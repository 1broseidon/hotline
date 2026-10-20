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
        crate::voice::room::RoomSide::new(Arc::downgrade(&room), vault),
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
        crate::voice::room::RoomSide::new(Arc::downgrade(&room), vault),
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
        first[0].contains("From your call (thread call:c1): the scheduler fix is in")
            && first[0].contains("and the release?"),
        "{}",
        first[0]
    );
    room.prompt("ada", "thanks", None, None).await.unwrap();
    let second = heard(&prompts, 2).await;
    assert!(!second[1].contains("scheduler fix"), "{}", second[1]);
}
