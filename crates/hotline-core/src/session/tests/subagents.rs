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
/// started reads as done: the reply is chat on the tape, not a step, and the
/// session says it is waiting on subagents while the turn is still running.
/// Tool work again is working again, and the reply after it lands as any
/// reply does. The phone hears the turn once, when it is over.
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
    assert!(
        lock(&room.push.sent).is_empty(),
        "the phone waits for the turn's end"
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
    let sent = pushed(&room, 1).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["body"], "Fixed both edge cases.");
}

/// Waits for the phone to have been sent `count` pushes, and a moment more
/// for one too many, then says what it was sent.
async fn pushed(room: &Room, count: usize) -> Vec<Value> {
    until(&format!("the phone is sent {count} pushes"), || {
        lock(&room.push.sent).len() >= count
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    lock(&room.push.sent).clone()
}

fn says(id: &str, text: &str) -> Update {
    Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    }
}

fn helper(id: &str) -> Update {
    Update::ToolCall {
        call_id: id.to_string(),
        title: format!("Compare tools, part {id}"),
        kind: crate::session::jobs::SUBAGENT.to_string(),
    }
}

fn reports(id: &str) -> Update {
    Update::ToolResult {
        call_id: id.to_string(),
        ok: true,
        output: format!("Part {id} compared."),
        images: Vec::new(),
    }
}

fn ends() -> Update {
    Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }
}

/// Waits until the turn has parked again after the agent said `id`.
async fn parked_after(room: &Room, id: &str) {
    until(&format!("the turn parks after {id}"), || {
        room.info("ada").awaiting_subagents
            && tape(room, "ada").iter().any(|event| event["id"] == id)
    })
    .await;
}

/// A teammate that starts four helpers and waits on them, saying a line as
/// some report, is one notification on the phone: nothing while the turn
/// waits, and when it is over, the last thing the teammate said in it.
#[tokio::test]
async fn a_turn_that_waits_on_its_helpers_reaches_the_phone_once_with_its_last_words() {
    let turn = vec![
        helper("h1"),
        helper("h2"),
        helper("h3"),
        helper("h4"),
        says("m-start", "Four helpers are comparing the tools."),
        Update::Parked,
        reports("h1"),
        says("m-one", "One of four is back."),
        Update::Parked,
        reports("h2"),
        says("m-two", "Two of four are back."),
        Update::Parked,
        reports("h3"),
        reports("h4"),
        says("m-done", "Comparison chart of 16 agent tools is ready."),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "helpers-one-push",
        Fake::new(Scripted::turns(vec![turn]).gated(gate.clone())),
    );
    pair_a_phone(&room);
    room.start("ada").await.unwrap();
    room.prompt("ada", "compare the agent tools", None, None)
        .await
        .unwrap();

    gate.add_permits(6);
    parked_after(&room, "m-start").await;
    gate.add_permits(3);
    parked_after(&room, "m-one").await;
    gate.add_permits(3);
    parked_after(&room, "m-two").await;
    assert!(
        lock(&room.push.sent).is_empty(),
        "nothing reaches the phone while the turn waits on its helpers: {:?}",
        lock(&room.push.sent)
    );

    gate.add_permits(4);
    until_state(&room, "ada", SessionState::Ready).await;
    let sent = pushed(&room, 1).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        sent[0]["body"],
        "Comparison chart of 16 agent tools is ready."
    );
    assert!(sent[0].get("categoryId").is_none(), "{sent:?}");
}

/// A card raised while the turn waits on its helpers needs the person now, so
/// it reaches the phone at once. A teammate that says nothing after the wait
/// has its line from before the wait heard when the turn is over.
#[tokio::test]
async fn a_card_in_the_wait_is_pushed_at_once_and_the_line_before_the_wait_at_the_end() {
    let turn = vec![
        helper("h1"),
        says("m-start", "A helper is checking the release."),
        Update::Parked,
        Update::Permission {
            request_id: "r1".to_string(),
            title: "Run cargo publish".to_string(),
            options: vec![PermissionOption {
                option_id: "once".to_string(),
                name: "Allow once".to_string(),
                kind: Some("allow_once".to_string()),
            }],
        },
        reports("h1"),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "helpers-card",
        Fake::new(Scripted::turns(vec![turn]).gated(gate.clone())),
    );
    pair_a_phone(&room);
    room.start("ada").await.unwrap();
    room.prompt("ada", "ship it", None, None).await.unwrap();

    gate.add_permits(3);
    parked_after(&room, "m-start").await;
    assert!(lock(&room.push.sent).is_empty());
    gate.add_permits(1);
    let sent = pushed(&room, 1).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["categoryId"], "permission");
    assert_eq!(sent[0]["body"], "Run cargo publish");

    gate.add_permits(2);
    until_state(&room, "ada", SessionState::Ready).await;
    let sent = pushed(&room, 2).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1]["body"], "A helper is checking the release.");
    assert!(sent[1].get("categoryId").is_none(), "{sent:?}");
}

/// A turn stopped while it waits on its helpers sends the phone nothing: its
/// line before the wait promised an answer that is not coming, and the
/// conversation says the turn was stopped. A restart's cut is the same stop.
#[tokio::test]
async fn a_turn_stopped_while_it_waits_on_its_helpers_pushes_nothing() {
    let turn = vec![
        helper("h1"),
        says("m-start", "A helper is checking the release."),
        Update::Parked,
        reports("h1"),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "helpers-stopped",
        Fake::new(
            Scripted::turns(vec![turn])
                .gated(gate.clone())
                .on_cancel(vec![Update::Turn {
                    stop_reason: "cancelled".to_string(),
                    usage: None,
                }]),
        ),
    );
    pair_a_phone(&room);
    room.start("ada").await.unwrap();
    room.prompt("ada", "check the release", None, None)
        .await
        .unwrap();

    gate.add_permits(3);
    parked_after(&room, "m-start").await;
    room.cancel("ada").unwrap();
    until_state(&room, "ada", SessionState::Ready).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        lock(&room.push.sent).is_empty(),
        "{:?}",
        lock(&room.push.sent)
    );
}

/// Nothing reaches the phone while the person is at the desk, a turn that
/// waited on its helpers included.
#[tokio::test]
async fn a_turn_that_waited_on_its_helpers_pushes_nothing_while_the_person_is_at_the_desk() {
    let turn = vec![
        helper("h1"),
        says("m-start", "A helper is checking the release."),
        Update::Parked,
        reports("h1"),
        says("m-done", "The release is fine."),
        ends(),
    ];
    let room = room("helpers-at-desk", Fake::new(Scripted::turns(vec![turn])));
    pair_a_phone(&room);
    room.desk_looking(true);
    room.start("ada").await.unwrap();
    room.prompt("ada", "check the release", None, None)
        .await
        .unwrap();
    until("the turn is over", || {
        room.info("ada").state == SessionState::Ready
            && tape(&room, "ada")
                .iter()
                .any(|event| event["kind"] == "turn")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(lock(&room.push.sent).is_empty());
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
