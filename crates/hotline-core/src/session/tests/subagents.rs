//! A turn left open for subagents, and work the agent started itself.

use super::*;

/// Waits for the teammate's session to say whether its turn is open only
/// for subagents.
async fn until_awaiting(room: &Room, persona_id: &str, want: bool) {
    until(&format!("awaiting_subagents becomes {want}"), || {
        room.info(persona_id).awaiting_subagents == want
    })
    .await;
}

/// A turn whose agent has said its reply and is open only for a subagent it
/// started reads as done: the reply is chat on the tape and on the phone, not
/// a step, and the session says it is waiting on subagents while the turn is
/// still running. Tool work again is working again, and the reply after it
/// lands as any reply does.
#[tokio::test]
async fn a_turn_left_open_only_for_subagents_reads_as_done_until_the_agent_works_again() {
    let says = |id: &str, text: &str| Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    };
    let turn = vec![
        Update::ToolCall {
            call_id: "c1".to_string(),
            title: "Read crane.rs".to_string(),
            kind: "read".to_string(),
        },
        Update::ToolResult {
            call_id: "c1".to_string(),
            ok: true,
            output: "fn lift()".to_string(),
            images: Vec::new(),
        },
        // Hotline Agent's subagent: its call stays running until the run
        // reports.
        Update::ToolCall {
            call_id: "s1".to_string(),
            title: "Check the edge cases".to_string(),
            kind: crate::session::jobs::SUBAGENT.to_string(),
        },
        says(
            "m-reply",
            "The lift is in crane.rs; a subagent is checking the edge cases.",
        ),
        Update::Parked,
        // The subagent reports, and the agent works on what it found.
        Update::ToolResult {
            call_id: "s1".to_string(),
            ok: true,
            output: "Two edge cases fail.".to_string(),
            images: Vec::new(),
        },
        Update::ToolCall {
            call_id: "c2".to_string(),
            title: "cargo test".to_string(),
            kind: "bash".to_string(),
        },
        Update::ToolResult {
            call_id: "c2".to_string(),
            ok: true,
            output: "ok".to_string(),
            images: Vec::new(),
        },
        says("m-after", "Fixed both edge cases."),
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "awaiting-subagents",
        Fake::new(Scripted::turns(vec![turn]).gated(gate.clone())),
    );
    std::fs::write(
        room.log.root().join("remote.json"),
        json!({
            "desktopId": "desk-1",
            "host": "desk.local",
            "enabled": true,
            "grants": [{
                "device": {"id": "phone-1", "name": "Phone", "pairedAt": 0, "publicKey": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},
                "push": {"token": "ExponentPushToken[phone]", "platform": "ios"}
            }]
        })
        .to_string(),
    )
    .unwrap();
    room.start("ada").await.unwrap();
    room.prompt("ada", "where is the lift", None, None)
        .await
        .unwrap();

    // Up to the park: the reply is chat, not a thought.
    gate.add_permits(5);
    until_awaiting(&room, "ada", true).await;
    assert_eq!(room.info("ada").state, SessionState::Thinking);
    let events = settled(&room, "ada", 4).await;
    let reply = events
        .iter()
        .find(|event| event["id"] == "m-reply")
        .expect("the reply is on the tape");
    assert_eq!(reply["kind"], "agent", "{events:?}");
    assert!(!events.iter().any(|event| event["kind"] == "turn"));
    let mut sent = Vec::new();
    for _ in 0..200 {
        sent = lock(&room.push.sent).clone();
        if !sent.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(sent.len(), 1, "the phone hears the reply now: {sent:?}");
    assert_eq!(
        sent[0]["body"],
        "The lift is in crane.rs; a subagent is checking the edge cases."
    );

    // The subagent's report wakes the agent, and the work is working.
    gate.add_permits(1);
    until_awaiting(&room, "ada", false).await;
    assert_eq!(room.info("ada").state, SessionState::Thinking);

    gate.add_permits(5);
    until_state(&room, "ada", SessionState::Ready).await;
    assert!(!room.info("ada").awaiting_subagents);
    let events = tape(&room, "ada");
    let after = events
        .iter()
        .find(|event| event["id"] == "m-after")
        .expect("the reply after the work is on the tape");
    assert_eq!(after["kind"], "agent");
    for _ in 0..200 {
        sent = lock(&room.push.sent).clone();
        if sent.len() > 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1]["body"], "Fixed both edge cases.");
}

/// A turn that parks with a command of its own still running is still at
/// work: what it said is its report, but it is not waiting only on
/// subagents. And a turn that never parks holds its last words as it
/// always has, until the turn ends.
#[tokio::test]
async fn a_turn_with_other_work_open_or_no_subagents_reads_as_working() {
    let says = |id: &str, text: &str| Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    };
    let command = vec![
        Update::ToolCall {
            call_id: "c1".to_string(),
            title: "cargo build".to_string(),
            kind: "shell".to_string(),
        },
        says("m-building", "The build is running."),
        Update::Parked,
        Update::ToolResult {
            call_id: "c1".to_string(),
            ok: true,
            output: "built".to_string(),
            images: Vec::new(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ];
    let plain = vec![
        Update::ToolCall {
            call_id: "c2".to_string(),
            title: "Read crane.rs".to_string(),
            kind: "read".to_string(),
        },
        Update::ToolResult {
            call_id: "c2".to_string(),
            ok: true,
            output: "fn lift()".to_string(),
            images: Vec::new(),
        },
        says("m-plain", "It is in crane.rs."),
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "working-not-awaiting",
        Fake::new(Scripted::turns(vec![command, plain]).gated(gate.clone())),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    gate.add_permits(3);
    let events = settled(&room, "ada", 3).await;
    assert_eq!(
        events.last().unwrap()["id"],
        "m-building",
        "the report is said: {events:?}"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!room.info("ada").awaiting_subagents);
    assert_eq!(room.info("ada").state, SessionState::Thinking);
    gate.add_permits(2);
    until_state(&room, "ada", SessionState::Ready).await;

    room.prompt("ada", "where is the lift", None, None)
        .await
        .unwrap();
    gate.add_permits(3);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !tape(&room, "ada")
            .iter()
            .any(|event| event["id"] == "m-plain"),
        "the last words wait for the turn's end"
    );
    assert!(!room.info("ada").awaiting_subagents);
    gate.add_permits(1);
    until_state(&room, "ada", SessionState::Ready).await;
    let events = tape(&room, "ada");
    let plain = events
        .iter()
        .find(|event| event["id"] == "m-plain")
        .expect("the reply lands at the turn's end");
    assert_eq!(plain["kind"], "agent");
}

/// What a teammate says about work it left running, once its turn is over,
/// reaches the chat and the phone when it is said, and the teammate is shown
/// working while it says it.
#[tokio::test]
async fn work_the_agent_started_itself_reaches_the_chat_and_the_phone() {
    let (scripted, unprompted) = Scripted::new(spoken_turn()).with_unprompted();
    let room = room("unprompted", Fake::new(scripted));
    std::fs::write(
        room.log.root().join("remote.json"),
        json!({
            "desktopId": "desk-1",
            "host": "desk.local",
            "enabled": true,
            "grants": [{
                "device": {"id": "phone-1", "name": "Phone", "pairedAt": 0, "publicKey": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},
                "push": {"token": "ExponentPushToken[phone]", "platform": "ios"}
            }]
        })
        .to_string(),
    )
    .unwrap();
    room.start("ada").await.unwrap();

    let (sender, updates) = mpsc::channel(8);
    unprompted.send(updates).unwrap();
    sender
        .send(Update::Message {
            kind: MessageKind::Agent,
            id: "m-bg".to_string(),
            text: "The build passed.".to_string(),
        })
        .await
        .unwrap();
    let mut working = false;
    for _ in 0..200 {
        if room.info("ada").state == SessionState::Thinking {
            working = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(working, "the teammate was never shown working");
    sender
        .send(Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        })
        .await
        .unwrap();
    drop(sender);

    let events = settled(&room, "ada", 1).await;
    assert!(
        events
            .iter()
            .any(|event| event["text"] == "The build passed."),
        "{events:?}"
    );
    let mut sent = Vec::new();
    for _ in 0..200 {
        sent = lock(&room.push.sent).clone();
        if !sent.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["body"], "The build passed.");
    for _ in 0..200 {
        if room.info("ada").state == SessionState::Ready {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the teammate never went back to ready");
}

/// A message sent while the teammate answers its own work waits for that
/// answer to end rather than being said over it.
#[tokio::test]
async fn a_message_waits_for_work_the_agent_started_itself() {
    let (scripted, unprompted) = Scripted::new(spoken_turn()).with_unprompted();
    let prompts = scripted.prompts.clone();
    let room = room("unprompted-waits", Fake::new(scripted));
    room.start("ada").await.unwrap();

    let (sender, updates) = mpsc::channel(8);
    unprompted.send(updates).unwrap();
    sender
        .send(Update::Message {
            kind: MessageKind::Agent,
            id: "m-bg".to_string(),
            text: "Still checking the logs.".to_string(),
        })
        .await
        .unwrap();
    settled(&room, "ada", 1).await;
    room.prompt("ada", "how is it going?", None, None)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        lock(&prompts).is_empty(),
        "the message was said over the agent's own work"
    );

    sender
        .send(Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        })
        .await
        .unwrap();
    drop(sender);
    for _ in 0..200 {
        if !lock(&prompts).is_empty() {
            assert_eq!(lock(&prompts).as_slice(), ["how is it going?"]);
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the message never reached the agent");
}
