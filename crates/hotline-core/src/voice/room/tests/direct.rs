use super::*;

/// A teammate's reply on a call to it has its spoken version said as it
/// streams, as one reply: one line on screen growing under one id, its audio
/// under that id ending in one empty final clip, and one line on the call's
/// thread. Neither the written version nor a tag reaches speech.
#[tokio::test]
async fn a_streamed_reply_says_its_spoken_version_as_one_line() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    let chunks = [
        "<spo",
        "ken>The build is still red. It's the fla",
        "ky config test again.</spok",
        "en>\n<written>| test | result |\n| config | flaky |\n</written>",
    ];
    for chunk in chunks {
        calls.reply_delta(&persona, "reply-1", chunk, &origin);
    }
    let whole = chunks.concat();
    assert!(calls.delivery(&persona, "reply-1", "Mack", &whole, true, Some(&origin)));
    let (said, clips) = one_reply(&mut rx).await;
    let line = &said[0].0;
    assert!(said.iter().all(|(id, _)| id == line), "{said:?}");
    let spoken = "The build is still red. It's the flaky config test again.";
    assert_eq!(
        said.iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        ["The build is still red.", spoken]
    );
    assert!(clips.iter().all(|(id, _, _)| id == line));
    assert_eq!(
        clips
            .iter()
            .map(|(_, index, empty)| (*index, *empty))
            .collect::<Vec<_>>(),
        [(0, false), (1, false), (2, true)]
    );
    assert_eq!(
        *lock(&fake.spoken),
        [
            "The build is still red.",
            "It's the flaky config test again."
        ]
    );
    // The turn's end hands the same reply over again: it is not said twice.
    assert!(calls.delivery(&persona, "reply-1", "Mack", &whole, true, Some(&origin)));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(lock(&fake.spoken).len(), 2);
    calls.end(&id).unwrap();
}

/// Each reply a call says is counted once, by how it was written, under the
/// model that wrote it; `voice.status` reads the counts back, and the desk
/// keeps them across a restart.
#[tokio::test]
async fn each_reply_said_is_counted_by_how_it_was_written() {
    let (_root, desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    assert_eq!(calls.status().replies, None);
    let origin = on_turn(&calls, &id, 1);
    for (event, reply) in [
        (
            "both",
            "<spoken>It's green.</spoken>\n<written>All 42 checks pass.</written>",
        ),
        ("spoken", "<spoken>It's green.</spoken>"),
        ("unclosed", "<spoken>It's green. <written>All 42 pass."),
        (
            "untagged",
            "All 42 checks pass:\n| check | result |\n| unit | ok |",
        ),
        // One short line, as before a tool, is said as written, uncounted.
        ("line", "Let me check the logs."),
    ] {
        assert!(calls.delivery(&persona, event, "Mack", reply, true, Some(&origin)));
        one_reply(&mut rx).await;
    }
    // Said again when the turn ends, a reply is not counted again.
    assert!(calls.delivery(
        &persona,
        "both",
        "Mack",
        "<spoken>It's green.</spoken>",
        true,
        Some(&origin)
    ));
    let model = calls.test_writer(&persona);
    assert!(model.starts_with("hotline"), "{model}");
    let counted = vec![crate::contract::VoiceReplies {
        model,
        both: 1,
        spoken_only: 1,
        unclosed: 1,
        untagged: 1,
        rewritten: 0,
    }];
    assert_eq!(calls.status().replies.as_ref(), Some(&counted));
    assert_eq!(
        hotline_voice::test_support::Replies::open(desk.log.root()).counts(),
        counted
    );
    let wire = serde_json::to_value(calls.status()).unwrap();
    assert_eq!(wire["replies"][0]["spokenOnly"], 1);
    calls.end(&id).unwrap();
}

/// A reply written only to be read, as an agent writes when it ignores the
/// tags.
const UNTAGGED: &str = "Here's the fix for the flaky test:\n```rust\nassert!(ready);\n```\nIt passes ten runs in a row now.";

/// The agent event the session writes for a reply, as it does just after
/// handing the reply to the call.
fn write_reply(desk: &crate::desk::Desk, persona: &str, event: &str, text: &str) {
    desk.log
        .append(
            &StreamId::Call(
                call_link(desk, persona).unwrap()["thread"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
            &json!({"kind": "agent", "id": event, "ts": 1, "text": text}),
        )
        .unwrap();
}

/// What was said for a reply, as its agent event keeps it.
fn kept_spoken(desk: &crate::desk::Desk, persona: &str, event: &str) -> Option<String> {
    desk.log
        .load(&StreamId::Call(
            call_link(desk, persona).unwrap()["thread"]
                .as_str()
                .unwrap()
                .into(),
        ))
        .into_iter()
        .find(|line| line["id"] == event)
        .and_then(|line| line["spoken"].as_str().map(str::to_string))
}

/// Legacy says its own opening while the model is still working, never a rewrite.
#[tokio::test]
async fn legacy_says_its_first_sentence_before_the_reply_finishes() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls.reply_delta(&persona, "reply-1", "Inspecting the flaky test. ", &origin);
    event(&mut rx, |event| matches!(event, VoiceEvent::Said { text, .. } if text == "Inspecting the flaky test.")).await;
    assert_eq!(*lock(&fake.spoken), ["Inspecting the flaky test."]);
    calls.reply_delta(&persona, "reply-1", "The race is fixed. ", &origin);
    assert!(calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "Inspecting the flaky test. The race is fixed.",
        true,
        Some(&origin)
    ));
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(
        said.last().unwrap().1,
        "Inspecting the flaky test. The race is fixed."
    );
    assert!(lock(&fake.rewrites).is_empty());
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn legacy_never_speaks_a_code_block_or_rewrites_its_own_opening() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    write_reply(&desk, &persona, "reply-1", UNTAGGED);
    assert!(calls.delivery(&persona, "reply-1", "Mack", UNTAGGED, true, Some(&origin)));
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(said.last().unwrap().1, "Here's the fix for the flaky test:");
    assert!(lock(&fake.rewrites).is_empty());
    assert_eq!(kept_spoken(&desk, &persona, "reply-1"), None);
    calls.end(&id).unwrap();
}

/// Speaking over a teammate stops what it is saying and gives the person the
/// floor, without stopping its turn: what it goes on writing of that reply is
/// not said, and the call takes the person's next words at once.
#[tokio::test]
async fn speaking_over_a_reply_stops_it_and_gives_the_person_the_floor() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            Ok(())
        })
        .unwrap();
    *lock(&fake.delay) = Duration::from_secs(5);
    calls.reply_delta(
        &persona,
        "reply-1",
        "<spoken>I looked at the logs. ",
        &origin,
    );
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "I looked at the logs."),
    )
    .await;
    calls.interrupt(&id).unwrap();
    event(&mut rx, |e| {
        matches!(
            e,
            VoiceEvent::State {
                state: VoiceState::Listening,
                ..
            }
        )
    })
    .await;
    calls.reply_delta(&persona, "reply-1", "The cache is stale.</spoken>", &origin);
    assert!(calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "<spoken>I looked at the logs. The cache is stale.</spoken>",
        true,
        Some(&origin)
    ));
    calls.reply_delta(
        &persona,
        "late-reply",
        "<spoken>More of the stale answer.</spoken>",
        &origin,
    );
    assert!(calls.delivery(
        &persona,
        "late-reply",
        "Mack",
        "More of the stale answer.",
        true,
        Some(&origin)
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(&event, VoiceEvent::Said { text, .. } if text.contains("stale")),
            "{event:?}"
        );
        assert!(!matches!(event, VoiceEvent::Clip { .. }), "{event:?}");
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let thread = call_link(&desk, &persona).unwrap()["thread"]
                .as_str()
                .unwrap()
                .to_string();
            let lines = desk.log.load(&StreamId::Call(thread));
            if let Some(cut) = lines.iter().find(|line| line["interrupted"] == true) {
                assert!(
                    cut["text"]
                        .as_str()
                        .unwrap()
                        .contains("I looked at the logs.")
                );
                assert!(cut["text"].as_str().unwrap().contains("do not resume"));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    *lock(&fake.delay) = Duration::ZERO;
    utterance(&calls, &id, 2).unwrap();
    calls.end(&id).unwrap();
}

/// The call thinks while the teammate's turn is open, between what it says,
/// and listens once the session has finished the turn. A turn that ends
/// before a later one the person said does not end the wait for that one.
#[tokio::test]
async fn the_call_thinks_while_the_teammate_works_and_listens_when_its_turn_ends() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    assert!(calls.delivery(
        &persona,
        "ack",
        "Mack",
        "<spoken>On it.</spoken>",
        true,
        Some(&origin)
    ));
    one_reply(&mut rx).await;
    event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await;
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            live_error: None,
            reconnecting: false,
            state: VoiceState::Thinking,
            reason: None,
            listening: true,
        },
        "still working, and taking what the person says"
    );
    calls
        .test_change(&id, |call| {
            call.seq = Some(2);
            call.answering = Some(2);
            Ok(())
        })
        .unwrap();
    calls.turn_ended(&persona, &origin);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        calls.test_change(&id, |call| Ok(call.answering)).unwrap(),
        Some(2)
    );
    let later = Origin { seq: 2, ..origin };
    calls.turn_ended(&persona, &later);
    event(&mut rx, |e| {
        matches!(
            e,
            VoiceEvent::State {
                state: VoiceState::Listening,
                ..
            }
        )
    })
    .await;
    calls.end(&id).unwrap();
}

/// While the teammate's turn works, the call takes what the person says: its
/// `thinking` says `listening`, an utterance is accepted and steers into the
/// turn, and it stops what the call was saying (barge-in). While the desk is
/// still taking the person's last words, `thinking` has the floor and says
/// no such thing; and a desk call, which has no teammate's turn, never does.
#[tokio::test]
async fn the_call_listens_while_the_teammate_works_and_speaking_over_it_cuts_it_off() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await,
        VoiceEvent::State {
            live_error: None,
            reconnecting: false,
            state: VoiceState::Thinking,
            reason: None,
            listening: true,
        }
    );
    // The wire carries it only when it is so.
    let wire = serde_json::to_value(calls.subscribe(&id).unwrap().0).unwrap();
    assert_eq!(wire["listening"], true);
    let listening = serde_json::to_value(VoiceEvent::State {
        live_error: None,
        reconnecting: false,
        state: VoiceState::Listening,
        reason: None,
        listening: false,
    })
    .unwrap();
    assert!(listening.get("listening").is_none(), "{listening}");

    // A reply begins mid-turn, and the person speaks over it.
    *lock(&fake.delay) = Duration::from_secs(5);
    calls.reply_delta(
        &persona,
        "narration",
        "<spoken>Looking at the logs now. ",
        &origin,
    );
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "Looking at the logs now."),
    )
    .await;
    let speaking = calls
        .test_change(&id, |call| Ok(call.speech.clone()))
        .unwrap();
    utterance(&calls, &id, 2).unwrap();
    assert!(speaking.is_cancelled(), "what the call was saying stops");
    assert!(
        calls
            .test_change(&id, |call| Ok(call.streams.is_empty()))
            .unwrap()
    );
    // Taking those words, the desk has the floor.
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            live_error: None,
            reconnecting: false,
            state: VoiceState::Thinking,
            reason: None,
            listening: false,
        }
    );
    *lock(&fake.delay) = Duration::ZERO;
    calls.end(&id).unwrap();

    // A desk call thinks with the floor.
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            listening: false,
            ..
        }
    ));
    assert!(
        calls
            .status()
            .capabilities
            .iter()
            .any(|capability| capability == LISTEN_WHILE_THINKING)
    );
    calls.end(&id).unwrap();
}

/// A reply its turn never finished, because the agent stopped mid-message,
/// is said as far as it got, and the call listens.
#[tokio::test]
async fn a_reply_the_turn_never_finished_is_said_as_far_as_it_got() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls.reply_delta(
        &persona,
        "reply-1",
        "<spoken>The deploy is half done. The rest",
        &origin,
    );
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "The deploy is half done."),
    )
    .await;
    calls.turn_ended(&persona, &origin);
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(said.last().unwrap().1, "The deploy is half done. The rest");
    event(&mut rx, |e| {
        matches!(
            e,
            VoiceEvent::State {
                state: VoiceState::Listening,
                ..
            }
        )
    })
    .await;
    calls.end(&id).unwrap();
}

/// A call that thinks says so again while it waits, so a phone does not take
/// a teammate's long work for a desk that went quiet.
#[tokio::test]
async fn a_call_that_thinks_says_so_again() {
    let (_root, _desk, calls, id, _persona, mut rx, _fake) = direct_call().await;
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await;
    // Nothing else says Thinking twice, so the second one is the timer's.
    assert_eq!(
        event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await,
        VoiceEvent::State {
            live_error: None,
            reconnecting: false,
            state: VoiceState::Thinking,
            reason: None,
            listening: true,
        }
    );
    calls.end(&id).unwrap();
}

/// A hold cuts off a reply being said, and the phone hears of it as of any
/// reply that comes while the call is held.
#[tokio::test]
async fn a_hold_cuts_off_a_reply_and_leaves_it_to_the_phone() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls.reply_delta(
        &persona,
        "reply-1",
        "<spoken>The deploy went out. ",
        &origin,
    );
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    calls.hold(&id, true).unwrap();
    calls.reply_delta(&persona, "reply-1", "Nothing broke.</spoken>", &origin);
    assert!(!calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "<spoken>The deploy went out. Nothing broke.</spoken>",
        true,
        Some(&origin)
    ));
    calls.end(&id).unwrap();
}

/// A call to a teammate has no call assistant, so a Chat budget that is off
/// cannot refuse it. Calling the desk lead has the same readiness.
#[tokio::test]
async fn a_direct_call_pays_for_no_call_assistant() {
    let free_speech = Arc::new(Fake {
        subscription: true,
        ..Fake::default()
    });
    let (_root, desk, calls) = desk(with_fake(free_speech));
    let persona = mack(&desk).await;
    spending(&desk, json!({"chat": {"dayUsd": 0}}));
    let status = calls.status();
    assert!(status.direct_available);
    assert!(status.available);
    let id = Uuid::new_v4().to_string();
    calls
        .start_target(&id, Some(persona), false, desk.clone())
        .unwrap();
    assert!(calls.test_ready_for(&id).is_ok());
    calls.end(&id).unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_legacy_turn_without_opening_speech_gets_one_sound_on_the_clip_wire() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.first_clip_started =
                Some(Instant::now().checked_sub(Duration::from_secs(3)).unwrap());
            Ok(())
        })
        .unwrap();
    let cue = event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Clip { id, .. } if id == "waiting-1"),
    )
    .await;
    if let VoiceEvent::Clip {
        mime_type,
        data,
        r#final,
        ..
    } = cue
    {
        assert_eq!(mime_type, "audio/wav");
        assert!(r#final);
        assert!(STANDARD.decode(data).unwrap().starts_with(b"RIFF"));
    }
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, VoiceEvent::Clip { .. } | VoiceEvent::Said { .. }),
            "no repeated sound or filler: {event:?}"
        );
    }
    calls.reply_delta(&persona, "reply-1", "<spoken>The tests pass. ", &origin);
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "The tests pass."),
    )
    .await;
    calls.end(&id).unwrap();
}
