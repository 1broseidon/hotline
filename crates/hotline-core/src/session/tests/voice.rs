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
        first[0].contains("From your call; this thread IS its transcript — read_thread reads it in full (thread call:c1): the scheduler fix is in")
            && first[0].contains("and the release?"),
        "{}",
        first[0]
    );
    room.prompt("ada", "thanks", None, None).await.unwrap();
    let second = heard(&prompts, 2).await;
    assert!(!second[1].contains("scheduler fix"), "{}", second[1]);
}

/// A reply said on a turn by turn call is said once, however many bubbles
/// the chat shows it in. The desk writes a long reply as bubbles (`reply`,
/// `reply-2`), and the later ones are the same reply, which the call has
/// already said as it streamed: they are never handed to it as replies of
/// their own, to be read out again from their first word.
#[tokio::test]
async fn a_reply_written_as_several_bubbles_is_said_on_the_call_once() {
    let first = "Here's what we covered this week. We built a status page and audited the board. We tested the approval and yes or no cards. We took the firewall up to the new release and sorted out the SSH trouble.";
    let second = "Three things are still open. The firewall jump is waiting on your go-ahead. I haven't confirmed the new teammate was added.";
    let reply = format!("<spoken>{first}\n\n{second}</spoken>");
    let turn = vec![
        Update::Delta {
            kind: MessageKind::Agent,
            message_id: "reply".into(),
            text: reply.clone(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: "reply".into(),
            text: reply,
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ];
    let room = room("voice-bubbles-said-once", Fake::new(Scripted::new(turn)));
    let fake = Arc::new(crate::voice::tests::Fake::default());
    *lock(&fake.transcript) = "What did we talk about this week?".into();
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
    let id = uuid::Uuid::new_v4().to_string();
    calls
        .start_target(
            &id,
            Some("ada".into()),
            false,
            Arc::new(crate::wire::tests::CoreHandle { room: room.clone() }),
        )
        .unwrap();
    use base64::Engine as _;
    calls
        .utterance(
            &id,
            1,
            "audio/wav",
            &base64::prelude::BASE64_STANDARD.encode(crate::voice::tests::wav()),
            1000,
        )
        .unwrap();
    let bubbles = || {
        room.log
            .load(&StreamId::of_thread(&ThreadId::call(&id)).unwrap())
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .count()
    };
    until("the reply is written as two bubbles", || bubbles() == 2).await;
    let said = || lock(&fake.spoken).join(" ");
    until("the whole reply is said", || {
        said().ends_with("new teammate was added.")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let said = said();
    assert_eq!(
        said.matches("Three things are still open.").count(),
        1,
        "{said}"
    );
    assert_eq!(
        said.matches("Here's what we covered this week.").count(),
        1,
        "{said}"
    );
    calls.end(&id).unwrap();
}
