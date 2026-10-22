//! Replies as chat: pacing into bubbles, thinking between tool calls, the
//! phone's report, and a reply typed or said on a call.

use super::*;

fn bubble(n: u32) -> String {
    format!("Paragraph {n} is long enough to stand as its own bubble in the chat.")
}

/// Two paragraphs become two bubbles with the driver's id on the first and
/// `{id}-2` on the second, same timestamp; five become five. The model said
/// one thing either way, so history reads one `Said::Agent`.
#[tokio::test]
async fn a_reply_is_paced_as_chat() {
    let first = bubble(1);
    let second = bubble(2);
    let two = format!("{first}\n\n{second}");
    let five_units: Vec<String> = (1..=5).map(bubble).collect();
    let five = five_units.join("\n\n");

    let room = room(
        "paced-chat",
        Fake::new(Scripted::turns(vec![
            saying("two", &two),
            saying("five", &five),
        ])),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "two bubbles", None, None).await.unwrap();
    let chat = settled(&room, "ada", 4).await;
    assert_eq!(
        kinds(&chat),
        ["user", "agent", "agent", "turn"],
        "two paragraphs are two agent events"
    );
    assert_eq!(chat[1]["id"], "m-two");
    assert_eq!(chat[2]["id"], "m-two-2");
    assert_eq!(chat[1]["ts"], chat[2]["ts"]);
    assert_eq!(chat[1]["text"], first);
    assert_eq!(chat[2]["text"], second);
    assert_eq!(
        words(said(&tape(&room, "ada"))),
        [Said::User("two bubbles".to_string()), Said::Agent(two),]
    );

    room.prompt("ada", "five bubbles", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 11).await;
    let agents: Vec<_> = events
        .iter()
        .filter(|event| {
            event["kind"] == "agent" && event["id"].as_str().unwrap().starts_with("m-five")
        })
        .collect();
    assert_eq!(agents.len(), 5, "{}", kinds(&events).join(", "));
    assert_eq!(agents[0]["id"], "m-five");
    assert_eq!(agents[1]["id"], "m-five-2");
    assert_eq!(agents[0]["ts"], agents[4]["ts"]);
    for (i, unit) in five_units.iter().enumerate() {
        assert_eq!(agents[i]["text"], *unit);
    }
    assert_eq!(
        words(said(&tape(&room, "ada"))),
        [
            Said::User("two bubbles".to_string()),
            Said::Agent(format!("{first}\n\n{second}")),
            Said::User("five bubbles".to_string()),
            Said::Agent(five),
        ]
    );
}

/// A teammate that narrates its work is heard once before it and once after.
/// What it said between its tool calls is on the tape as thinking, its
/// deltas after the acknowledgement stream as thinking, and the phone is
/// told about the two lines that were said and nothing else.
#[tokio::test]
async fn what_is_said_between_tool_calls_is_thinking_not_chat() {
    fn says(id: &str, text: &str) -> [Update; 2] {
        [
            Update::Delta {
                kind: MessageKind::Agent,
                message_id: id.to_string(),
                text: text.to_string(),
            },
            Update::Message {
                kind: MessageKind::Agent,
                id: id.to_string(),
                text: text.to_string(),
            },
        ]
    }
    fn calls(id: &str) -> [Update; 2] {
        [
            Update::ToolCall {
                call_id: id.to_string(),
                title: format!("cargo {id}"),
                kind: "bash".to_string(),
            },
            Update::ToolResult {
                call_id: id.to_string(),
                ok: true,
                output: "ok".to_string(),
                images: Vec::new(),
            },
        ]
    }
    let mut turn = Vec::new();
    turn.extend(says("m-ack", "on it"));
    turn.extend(calls("c1"));
    turn.extend(says("m-mid", "Found the failing test, fixing it now."));
    turn.extend(calls("c2"));
    turn.extend(says("m-again", "Running the suite again."));
    turn.extend(calls("c3"));
    turn.extend(says("m-done", "Done, all green."));
    turn.push(Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    });

    let room = room("narration", Fake::new(Scripted::turns(vec![turn])));
    room.start("ada").await.unwrap();
    let mut deltas = room.subscribe_deltas();
    room.prompt("ada", "fix the build", None, None)
        .await
        .unwrap();
    let chat = settled(&room, "ada", 9).await;
    assert_eq!(
        kinds(&chat),
        [
            "user", "agent", "tool", "thought", "tool", "thought", "tool", "agent", "turn"
        ],
        "{}",
        kinds(&chat).join(", ")
    );
    assert_eq!(chat[1]["text"], "on it");
    assert_eq!(chat[3]["text"], "Found the failing test, fixing it now.");
    assert_eq!(chat[3]["id"], "m-mid", "the words are demoted, not lost");
    assert_eq!(chat[5]["text"], "Running the suite again.");
    assert_eq!(chat[7]["text"], "Done, all green.");
    assert_eq!(
        words(said(&tape(&room, "ada"))),
        [
            Said::User("fix the build".to_string()),
            Said::Agent("on it\n\nDone, all green.".to_string()),
        ],
        "the model's own history keeps what was said, not what was thought"
    );

    let mut streamed = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        streamed.push(match delta {
            StreamDelta::ThreadDelta {
                message_id,
                kind: DeltaKind::Text,
                ..
            } => format!("agent:{message_id}"),
            StreamDelta::ThreadDelta {
                message_id,
                kind: DeltaKind::Thought,
                ..
            } => format!("thought:{message_id}"),
            other => panic!("the room broadcast {other:?} to the DM"),
        });
    }
    assert_eq!(
        streamed,
        [
            "agent:m-ack",
            "thought:m-mid",
            "thought:m-again",
            "thought:m-done"
        ],
        "after the acknowledgement the window shows thinking, never a bubble that vanishes"
    );
}

/// A turn is one notification on the phone however many bubbles it took:
/// the report, once the driver is done with the line.
#[tokio::test]
async fn a_turn_reaches_the_phone_once_with_its_report() {
    let says = |id: &str, text: &str| Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    };
    let calls = |id: &str| {
        [
            Update::ToolCall {
                call_id: id.to_string(),
                title: format!("cargo {id}"),
                kind: "bash".to_string(),
            },
            Update::ToolResult {
                call_id: id.to_string(),
                ok: true,
                output: "ok".to_string(),
                images: Vec::new(),
            },
        ]
    };
    let mut turn = vec![says("m-ack", "on it")];
    turn.extend(calls("c1"));
    turn.push(says("m-mid", "Found the failing test, fixing it now."));
    turn.extend(calls("c2"));
    turn.push(says("m-done", "Done, all green."));
    turn.push(Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    });
    let room = room("one-glance", Fake::new(Scripted::turns(vec![turn])));
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
    room.prompt("ada", "fix the build", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 7).await;
    let mut sent = Vec::new();
    for _ in 0..200 {
        sent = lock(&room.push.sent).clone();
        if !sent.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["body"], "Done, all green.");
    assert_eq!(sent[0]["data"]["personaId"], "ada");
}

/// An agent asked once on a call for a spoken version and a written one keeps
/// that in its own history, and may write the tags in a typed reply long
/// after. The chat, the tape and the phone have the written version and never
/// a tag, even when one arrives split across chunks, and nothing was said, so
/// nothing is kept as said.
#[tokio::test]
async fn a_typed_reply_is_shown_kept_and_pushed_as_its_written_version() {
    let delta = |text: &str| Update::Delta {
        kind: MessageKind::Agent,
        message_id: "m1".to_string(),
        text: text.to_string(),
    };
    let room = room(
        "typed-tags",
        Fake::new(Scripted::new(vec![
            delta("<spoken>It's green.</spoken>\n<"),
            delta("written>All 42 checks pass, so the build is green.\n</wri"),
            delta("tten>"),
            Update::Message {
                kind: MessageKind::Agent,
                id: "m1".to_string(),
                text: "<spoken>It's green.</spoken>\n<written>All 42 checks pass, so the build is green.\n</written>".to_string(),
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ])),
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
    let mut deltas = room.subscribe_deltas();
    room.start("ada").await.unwrap();
    room.prompt("ada", "Is the build green?", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "Is the build green?");
    assert_eq!(
        events[1]["text"],
        "All 42 checks pass, so the build is green."
    );
    assert!(events[1].get("spoken").is_none(), "{}", events[1]);

    let mut streamed = String::new();
    while let Ok(delta) = deltas.try_recv() {
        if let StreamDelta::ThreadDelta { text, .. } = delta {
            streamed.push_str(&text);
        }
    }
    assert_eq!(streamed, "All 42 checks pass, so the build is green.\n");

    let mut sent = Vec::new();
    for _ in 0..200 {
        sent = lock(&room.push.sent).clone();
        if !sent.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        sent[0]["body"],
        "All 42 checks pass, so the build is green."
    );
}

/// The line an agent writes before its tools stays its own in the history,
/// and the bubbles of a reply said on a call stay inside its written version.
#[test]
fn a_rebuilt_history_shows_both_versions_of_a_reply_said_on_a_call() {
    let tape = [
        json!({"kind": "user", "id": "u1", "text": "Check the build."}),
        json!({"kind": "agent", "id": "a1", "text": "On it."}),
        json!({"kind": "agent", "id": "a2", "text": "Two fail:", "spoken": "Two tests fail."}),
        json!({"kind": "agent", "id": "a2-2", "text": "| test | result |"}),
        json!({"kind": "user", "id": "u2", "text": "Which?"}),
        json!({"kind": "agent", "id": "a3", "text": "The config ones."}),
    ];
    assert_eq!(
        said(&tape),
        [
            Said::User("Check the build.".to_string()),
            Said::Agent(
                "On it.\n\n<spoken>Two tests fail.</spoken>\n<written>Two fail:\n\n| test | result |</written>"
                    .to_string()
            ),
            Said::User("Which?".to_string()),
            Said::Agent("The config ones.".to_string()),
        ]
    );
}

#[test]
fn conversational_call_text_is_its_spoken_words() {
    let events = crate::session::prompt::event_of(
        Update::Message {
            kind: MessageKind::Agent,
            id: "answer".into(),
            text:
                "<spoken>Yes, that works.</spoken><written>Yes, I think that should work.</written>"
                    .into(),
        },
        &mut HashMap::new(),
        true,
        false,
    );
    assert_eq!(events.len(), 1);
    let event = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(event["text"], "Yes, that works.");
    assert_eq!(event["spoken"], event["text"]);
}
