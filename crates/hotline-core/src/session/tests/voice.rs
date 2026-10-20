//! Replies on a voice handoff and on a direct call.

use super::*;

#[tokio::test]
async fn short_voice_handoff_replies_keep_their_text_without_an_extra_model() {
    let turn = vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "reply".into(),
            text: "The checks passed.".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ];
    let room = room("voice-push-provenance", Fake::new(Scripted::new(turn)));
    std::fs::write(room.log.root().join("remote.json"), json!({"desktopId":"desk", "host":"desk.local", "enabled":true, "grants":[{"device":{"id":"phone","name":"Phone","pairedAt":0,"publicKey":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},"push":{"token":"ExponentPushToken[phone]","platform":"ios"}}]}).to_string()).unwrap();
    let fake = Arc::new(crate::voice::tests::Fake::default());
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(
            room.log.root(),
            room.log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let calls = crate::voice::Calls::new(
        room.log.clone(),
        vault,
        Arc::downgrade(&room),
        Some(crate::voice::tests::with_fake(fake.clone())),
    );
    room.set_voice(&calls);
    room.start("ada").await.unwrap();
    for (index, from_voice) in [false, true, false].into_iter().enumerate() {
        if from_voice {
            crate::wire::commands::VOICE_COMMAND
                .scope((), room.prompt("ada", "Check the PR.", None, None))
                .await
                .unwrap();
        } else {
            room.prompt("ada", "Check the PR.", None, None)
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if lock(&room.push.sent).len() > index {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    let sent = lock(&room.push.sent).clone();
    assert_eq!(sent[0]["body"], "The checks passed.");
    assert_eq!(sent[1]["body"], "The checks passed.");
    assert_eq!(sent[2]["body"], "The checks passed.");
    assert_eq!(fake.narrations.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        tape(&room, "ada")
            .iter()
            .filter(|e| e["kind"] == "user" && e["id"].as_str().unwrap().starts_with("voice:"))
            .count(),
        1
    );
}

/// The paid summary is the voice handoff's alone: a typed prompt with the
/// same long reply is pushed as written, and asks no model.
#[tokio::test]
async fn a_long_reply_is_summarised_for_its_push_only_when_a_voice_handoff_asked_for_it() {
    let long = "The checks passed. The build is green. The branch is ready to merge.";
    let turn = vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "reply".into(),
            text: long.into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ];
    let room = room("voice-push-summary", Fake::new(Scripted::new(turn)));
    std::fs::write(room.log.root().join("remote.json"), json!({"desktopId":"desk", "host":"desk.local", "enabled":true, "grants":[{"device":{"id":"phone","name":"Phone","pairedAt":0,"publicKey":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},"push":{"token":"ExponentPushToken[phone]","platform":"ios"}}]}).to_string()).unwrap();
    let fake = Arc::new(crate::voice::tests::Fake::default());
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(
            room.log.root(),
            room.log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let calls = crate::voice::Calls::new(
        room.log.clone(),
        vault,
        Arc::downgrade(&room),
        Some(crate::voice::tests::with_fake(fake.clone())),
    );
    room.set_voice(&calls);
    room.start("ada").await.unwrap();
    for (index, from_voice) in [false, true, false].into_iter().enumerate() {
        if from_voice {
            crate::wire::commands::VOICE_COMMAND
                .scope((), room.prompt("ada", "Check the PR.", None, None))
                .await
                .unwrap();
        } else {
            room.prompt("ada", "Check the PR.", None, None)
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if lock(&room.push.sent).len() > index {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    let sent = lock(&room.push.sent).clone();
    assert_eq!(sent[0]["body"], long);
    assert_eq!(sent[1]["body"], format!("Ada says: {long}"));
    assert_eq!(sent[2]["body"], long);
    assert_eq!(fake.narrations.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_direct_call_speaks_committed_acknowledgements_and_hangup_keeps_work_running() {
    let gate = Arc::new(Semaphore::new(0));
    let driver = Scripted::new(vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "ack".into(),
            text: "On it.".into(),
        },
        Update::ToolCall {
            call_id: "tool".into(),
            title: "Check".into(),
            kind: "test".into(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: "thinking".into(),
            text: "Internal narration.".into(),
        },
        Update::ToolCall {
            call_id: "other-tool".into(),
            title: "Verify".into(),
            kind: "test".into(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: "report".into(),
            text: "All checks passed.".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ])
    .gated(gate.clone());
    let cancels = driver.cancels.clone();
    let prompts = driver.prompts.clone();
    let room = room("direct-voice-committed", Fake::new(driver));
    let fake = Arc::new(crate::voice::tests::Fake::default());
    *lock(&fake.transcript) = "Check the changes.".into();
    let store = Arc::new(crate::credentials::MemoryStore::default());
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(room.log.root(), room.log.clone(), store.clone())
            .unwrap(),
    );
    let calls = crate::voice::Calls::new(
        room.log.clone(),
        vault,
        Arc::downgrade(&room),
        Some(crate::voice::tests::with_fake(fake.clone())),
    );
    room.set_voice(&calls);
    // Only the dispatcher's handle is needed here; direct routing uses the room
    // above, with its scripted real session and standing capability lease.
    let handle_root = tempfile::tempdir().unwrap();
    let handle = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            handle_root.path(),
            store,
            Some(crate::voice::tests::with_fake(fake.clone())),
        )
        .unwrap(),
    );
    let call = uuid::Uuid::new_v4().to_string();
    calls
        .start_target(&call, Some("ada".into()), true, handle)
        .unwrap();
    let (_, mut events) = calls.subscribe(&call).unwrap();
    let mut data = b"RIFF".to_vec();
    data.extend(32_036u32.to_le_bytes());
    data.extend(b"WAVEfmt ");
    data.extend(16u32.to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(16_000u32.to_le_bytes());
    data.extend(32_000u32.to_le_bytes());
    data.extend(2u16.to_le_bytes());
    data.extend(16u16.to_le_bytes());
    data.extend(b"data");
    data.extend(32_000u32.to_le_bytes());
    data.resize(32_044, 0);
    calls
        .utterance(
            &call,
            1,
            "audio/wav",
            &base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data),
            1000,
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while lock(&prompts).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let existing_session = room.session("ada").unwrap();
    gate.add_permits(2);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                crate::contract::VoiceEvent::Clip { .. }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(*lock(&fake.spoken), ["On it."]);
    assert_eq!(room.info("ada").state, SessionState::Thinking);
    calls.hold(&call, true).unwrap();
    calls.interrupt(&call).unwrap();
    calls.end(&call).unwrap();
    assert_eq!(*lock(&cancels), 0);
    gate.add_permits(4);
    until_state(&room, "ada", SessionState::Ready).await;
    assert!(Arc::ptr_eq(
        &existing_session,
        &room.session("ada").unwrap()
    ));
    assert_eq!(
        *lock(&fake.spoken),
        ["On it."],
        "no late speech after hangup"
    );
    let lines = tape(&room, "ada");
    assert!(
        lines
            .iter()
            .any(|line| line["kind"] == "thought" && line["text"] == "Internal narration.")
    );
    assert!(
        lines
            .iter()
            .any(|line| line["kind"] == "agent" && line["text"] == "All checks passed.")
    );
    assert_eq!(
        fake.answers.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "direct calls bypass the dispatcher"
    );
}

#[tokio::test]
async fn unconsumed_direct_call_steering_keeps_its_origin_when_replayed() {
    let gate = Arc::new(Semaphore::new(0));
    let unconsumed = Arc::new(Mutex::new(Vec::new()));
    let end = Update::Turn {
        stop_reason: "end_turn".into(),
        usage: None,
    };
    let mut driver = Scripted::turns(vec![
        vec![end.clone()],
        vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "replayed-reply".into(),
                text: "The updated request is done.".into(),
            },
            end,
        ],
    ])
    .gated(gate.clone());
    driver.unconsumed = Some(unconsumed.clone());
    let prompts = driver.prompts.clone();
    let room = room("direct-voice-steering-replay", Fake::new(driver));
    room.start("ada").await.unwrap();
    room.prompt("ada", "Start the original request.", None, None)
        .await
        .unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;

    let fake = Arc::new(crate::voice::tests::Fake::default());
    *lock(&fake.transcript) = "Update the request.".into();
    let store = Arc::new(crate::credentials::MemoryStore::default());
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(room.log.root(), room.log.clone(), store.clone())
            .unwrap(),
    );
    let calls = crate::voice::Calls::new(
        room.log.clone(),
        vault,
        Arc::downgrade(&room),
        Some(crate::voice::tests::with_fake(fake.clone())),
    );
    room.set_voice(&calls);
    let handle_root = tempfile::tempdir().unwrap();
    let handle = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            handle_root.path(),
            store,
            Some(crate::voice::tests::with_fake(fake.clone())),
        )
        .unwrap(),
    );
    let call = uuid::Uuid::new_v4().to_string();
    calls
        .start_target(&call, Some("ada".into()), true, handle)
        .unwrap();
    let (_, mut events) = calls.subscribe(&call).unwrap();
    calls
        .audio(
            &call,
            1,
            0,
            &base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0; 320]),
            true,
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while lock(&unconsumed).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        words_of(&lock(&unconsumed)[0].0),
        crate::voice::spoken::voice_turn("Update the request.", false, false)
    );
    gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), async {
        while lock(&prompts).len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        *lock(&prompts),
        [
            "Start the original request.".to_string(),
            crate::voice::spoken::voice_turn("Update the request.", false, false)
        ]
    );
    gate.add_permits(2);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let crate::contract::VoiceEvent::Clip { .. } = events.recv().await.unwrap() {
                break;
            }
        }
    })
    .await
    .unwrap();
    until_state(&room, "ada", SessionState::Ready).await;
    assert_eq!(*lock(&fake.spoken), ["The updated request is done."]);
    let voice_lines: Vec<_> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "user" && event["text"] == "Update the request.")
        .collect();
    assert_eq!(voice_lines.len(), 1, "replay does not rewrite the input");
    assert_eq!(
        crate::voice::Origin::from_event_id(voice_lines[0]["id"].as_str().unwrap()),
        Some(crate::voice::Origin {
            call_id: call.clone(),
            seq: 1,
            direct: true,
            front: false,
            ended: false,
        })
    );
    calls.end(&call).unwrap();
}

/// A call's post is the teammate's message in the chat, stamped as from the
/// call. The person asked for the call, so the home agent is not woken: its
/// live session is handed the post ahead of the person's next line, once.
#[tokio::test]
async fn a_calls_post_rides_on_the_home_sessions_next_line_once() {
    let driver = Scripted::new(spoken_turn());
    let prompts = driver.prompts.clone();
    let room = room("call-post-handed", Fake::new(driver));
    pair_a_phone(&room);
    room.start("ada").await.unwrap();

    room.post(
        "ada",
        &ThreadId::call("c1"),
        crate::session::Post::words("the scheduler fix is in"),
    )
    .unwrap();
    let posted = tape(&room, "ada");
    let posted = posted.last().unwrap();
    assert_eq!(posted["kind"], "agent");
    assert_eq!(posted["text"], "the scheduler fix is in");
    assert_eq!(posted["from"], json!({"thread": "c1", "kind": "call"}));
    assert_eq!(lock(&prompts).len(), 0, "the home agent is not woken");
    let pushed = room.sent_voice_pushes();
    assert_eq!(pushed.len(), 1, "no call is up, so the person is told");
    assert_eq!(pushed[0]["body"], "the scheduler fix is in");

    room.prompt("ada", "and the release?", None, None)
        .await
        .unwrap();
    let first = heard(&prompts, 1).await;
    assert!(
        first[0].contains("From your call: the scheduler fix is in")
            && first[0].contains("and the release?"),
        "{}",
        first[0]
    );
    room.prompt("ada", "thanks", None, None).await.unwrap();
    let second = heard(&prompts, 2).await;
    assert!(!second[1].contains("scheduler fix"), "{}", second[1]);
}
