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
        [
            "The build is still red.",
            spoken,
            &format!("{spoken} The result is available in our chat.")
        ]
    );
    assert!(clips.iter().all(|(id, _, _)| id == line));
    assert_eq!(
        clips
            .iter()
            .map(|(_, index, empty)| (*index, *empty))
            .collect::<Vec<_>>(),
        [(0, false), (1, false), (2, false), (3, true)]
    );
    assert_eq!(
        *lock(&fake.spoken),
        [
            "The build is still red.",
            "It's the flaky config test again.",
            "The result is available in our chat."
        ]
    );
    // The turn's end hands the same reply over again: it is not said twice.
    assert!(calls.delivery(&persona, "reply-1", "Mack", &whole, true, Some(&origin)));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(lock(&fake.spoken).len(), 3);
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

/// Turn by turn says its own opening while the model is still working, never a rewrite.
#[tokio::test]
async fn turn_by_turn_says_its_first_sentence_before_the_reply_finishes() {
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
async fn turn_by_turn_never_speaks_a_code_block_or_rewrites_its_own_opening() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    write_reply(&desk, &persona, "reply-1", UNTAGGED);
    assert!(calls.delivery(&persona, "reply-1", "Mack", UNTAGGED, true, Some(&origin)));
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(
        said.last().unwrap().1,
        "Here's the fix for the flaky test: The result is available in our chat."
    );
    assert!(lock(&fake.rewrites).is_empty());
    tokio::time::timeout(Duration::from_secs(15), async {
        while kept_spoken(&desk, &persona, "reply-1").is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        kept_spoken(&desk, &persona, "reply-1").as_deref(),
        Some("Here's the fix for the flaky test: The result is available in our chat.")
    );
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
async fn a_turn_by_turn_reply_without_opening_speech_gets_one_sound_on_the_clip_wire() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.feedback_at = Some(tokio::time::Instant::now());
            Ok(())
        })
        .unwrap();
    for _ in 0..31 {
        tokio::time::advance(Duration::from_millis(250)).await;
        tokio::task::yield_now().await;
        while let Ok(event) = rx.try_recv() {
            assert!(
                !matches!(event, VoiceEvent::Clip { .. } | VoiceEvent::Said { .. }),
                "no early waiting sound before 8s: {event:?}"
            );
        }
    }
    tokio::time::advance(Duration::from_millis(500)).await;
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
    for _ in 0..30 {
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        while let Ok(event) = rx.try_recv() {
            assert!(
                !matches!(event, VoiceEvent::Clip { .. } | VoiceEvent::Said { .. }),
                "no repeated sound or canned filler: {event:?}"
            );
        }
    }
    calls.reply_delta(&persona, "reply-1", "<spoken>The tests pass. ", &origin);
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "The tests pass."),
    )
    .await;
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn narrator_points_at_real_results_only_once_every_three_turns() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    for (index, written) in [
        "Yes, that works.",
        "```rust\nlet ready = true;\n```",
        "| job | result |\n| --- | --- |\n| unit | ok |",
        "- first\n- second",
        "| job | result |\n| --- | --- |\n| unit | ok |",
    ]
    .into_iter()
    .enumerate()
    {
        let seq = u32::try_from(index + 1).unwrap();
        let origin = on_turn(&calls, &id, seq);
        let reply = format!("<spoken>Answer {seq}.</spoken><written>{written}</written>");
        assert!(calls.delivery(
            &persona,
            &format!("reply-{seq}"),
            "Mack",
            &reply,
            true,
            Some(&origin)
        ));
        let (said, _) = one_reply(&mut rx).await;
        let pointer = said.last().unwrap().1.contains("available in our chat");
        assert_eq!(pointer, seq == 2 || seq == 5, "turn {seq}: {said:?}");
    }
    let instructions = lock(&fake.pointer_instructions);
    assert_eq!(instructions.len(), 2);
    // The count that spaces the pointers is the desk's: it is not in what the model reads.
    assert!(!instructions[1].contains("turns in"));
    assert!(instructions[1].contains("Answer 4."));
    calls.end(&id).unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_text_opening_does_not_suppress_the_cue_while_tts_is_stalled() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    *lock(&fake.delay) = Duration::from_secs(10);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.feedback_at = Some(tokio::time::Instant::now());
            Ok(())
        })
        .unwrap();
    calls.reply_delta(&persona, "opening", "I'll check the lighthouse. ", &origin);
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    tokio::time::advance(Duration::from_millis(8_250)).await;
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Clip { id, .. } if id == "waiting-1"),
    )
    .await;
    calls.end(&id).unwrap();
}

#[tokio::test(start_paused = true)]
async fn explained_work_stays_silent_and_never_repeats_canned_progress() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.feedback_at = Some(tokio::time::Instant::now());
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    // Teammate speaks a scoped opening line before tools run.
    let opening = "<spoken>Let me check the logs, give me a moment.</spoken><written>Checking logs.</written>";
    assert!(calls.delivery(&persona, "opening-1", "Mack", opening, true, Some(&origin)));
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(said.len(), 1);
    assert_eq!(lock(&fake.spoken).len(), 1);
    // Tools run quietly for 80 seconds: no 8s waiting chime and no canned 20s progress.
    for _ in 0..80 {
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        while let Ok(event) = rx.try_recv() {
            assert!(
                !matches!(event, VoiceEvent::Clip { .. } | VoiceEvent::Said { .. }),
                "explained work stays silent while tools run: {event:?}"
            );
        }
    }
    assert_eq!(lock(&fake.spoken).len(), 1);
    calls.turn_ended(&persona, &origin);
    calls.end(&id).unwrap();
}

#[tokio::test(start_paused = true)]
async fn pipelined_whole_clip_tts_preserves_sentence_order() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    *lock(&fake.delay) = Duration::from_millis(150);
    let reply = "<spoken>First sentence is here. Second sentence follows right behind. Third closes cleanly.</spoken><written>Done.</written>";
    assert!(calls.delivery(&persona, "pipe-1", "Mack", reply, true, Some(&origin)));
    let mut indices = Vec::new();
    let mut saw_final = false;
    for _ in 0..40 {
        tokio::time::advance(Duration::from_millis(50)).await;
        tokio::task::yield_now().await;
        while let Ok(event) = rx.try_recv() {
            if let VoiceEvent::Clip {
                index,
                r#final,
                data,
                ..
            } = event
            {
                indices.push((index, !data.is_empty(), r#final));
                if r#final {
                    saw_final = true;
                }
            }
        }
        if saw_final {
            break;
        }
    }
    assert!(saw_final, "expected final clip, got {indices:?}");
    assert_eq!(
        indices,
        vec![
            (0, true, false),
            (1, true, false),
            (2, true, false),
            (3, false, true)
        ]
    );
    assert_eq!(
        *lock(&fake.spoken),
        vec![
            "First sentence is here.",
            "Second sentence follows right behind.",
            "Third closes cleanly.",
        ]
    );
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn ollama_tts_is_prewarmed_on_turn_by_turn_call_start_without_spending_budget() {
    let fake = Arc::new(Fake {
        provider_id: Some("ollama"),
        ..Fake::default()
    });
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    until("ollama pre-warm", || !lock(&fake.spoken).is_empty()).await;
    assert_eq!(*lock(&fake.spoken), vec!["ok"]);
    let status = calls.status();
    assert_eq!(status.budget.spent_day_usd, 0.0);
    calls.end(&id).unwrap();
}

/// Moves the paused clock on a second at a time, draining what the call sends.
async fn quietly(rx: &mut broadcast::Receiver<VoiceEvent>, seconds: u64) {
    for _ in 0..seconds {
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        while rx.try_recv().is_ok() {}
    }
}

fn nudged(calls: &Calls, id: &str) -> bool {
    calls
        .test_change(id, |call| Ok(call.overrun_nudged))
        .unwrap()
}

/// A turn that said what it is doing is left one overrun note 90 seconds
/// after it last spoke, and never another however long the work runs.
#[tokio::test(start_paused = true)]
async fn the_overrun_note_comes_once_90_seconds_after_the_last_spoken_line() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.feedback_at = Some(tokio::time::Instant::now());
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    let opening = "<spoken>Let me inspect the logs, give me a couple of minutes.</spoken><written>Inspecting logs.</written>";
    assert!(calls.delivery(
        &persona,
        "open-overrun",
        "Mack",
        opening,
        true,
        Some(&origin)
    ));
    let _ = one_reply(&mut rx).await;
    quietly(&mut rx, 89).await;
    assert!(!nudged(&calls, &id), "not before 90 seconds of quiet");
    quietly(&mut rx, 2).await;
    assert!(nudged(&calls, &id), "one note after 90 seconds of quiet");
    // Long past what used to be a 180 second cooldown: still the one note,
    // which only the person's next turn clears.
    quietly(&mut rx, 400).await;
    assert!(nudged(&calls, &id));
    calls.turn_ended(&persona, &origin);
    calls.end(&id).unwrap();
}

/// A turn that went straight to its tools, with no opening line, still gets
/// its one note: 90 seconds after the tone that covered its silence.
#[tokio::test(start_paused = true)]
async fn a_tool_first_turn_gets_the_overrun_note_too() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .test_change(&id, |call| {
            call.answering = Some(1);
            call.feedback_at = Some(tokio::time::Instant::now());
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    tokio::time::advance(Duration::from_millis(8_250)).await;
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Clip { id, .. } if id == "waiting-1"),
    )
    .await;
    quietly(&mut rx, 88).await;
    assert!(!nudged(&calls, &id), "90 seconds counted from the tone");
    quietly(&mut rx, 3).await;
    assert!(
        nudged(&calls, &id),
        "a silent tool-first turn gets its note"
    );
    calls.turn_ended(&persona, &origin);
    calls.end(&id).unwrap();
}

/// Hold, the person cutting in and the turn ending each take the note off:
/// nothing is left for work the person is not waiting on.
#[tokio::test(start_paused = true)]
async fn hold_interrupt_and_a_finished_turn_leave_no_overrun_note() {
    for how in ["hold", "interrupt", "ended"] {
        let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
        let origin = on_turn(&calls, &id, 1);
        calls
            .test_change(&id, |call| {
                call.answering = Some(1);
                call.feedback_at = Some(tokio::time::Instant::now());
                call.state(VoiceState::Thinking, None);
                Ok(())
            })
            .unwrap();
        quietly(&mut rx, 30).await;
        match how {
            "hold" => calls.hold(&id, true).unwrap(),
            "interrupt" => calls.interrupt(&id).unwrap(),
            _ => calls.turn_ended(&persona, &origin),
        }
        quietly(&mut rx, 300).await;
        assert!(!nudged(&calls, &id), "{how} leaves no note");
        calls.end(&id).unwrap();
    }
}

/// Two sentences synthesizing at once, then the person cuts in or hangs up:
/// neither of them, nor anything after, is played.
#[tokio::test(start_paused = true)]
async fn cutting_in_or_hanging_up_mid_synthesis_plays_nothing_more() {
    for hang_up in [false, true] {
        let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
        let origin = on_turn(&calls, &id, 1);
        calls
            .test_change(&id, |call| {
                call.answering = Some(1);
                Ok(())
            })
            .unwrap();
        *lock(&fake.delay) = Duration::from_secs(1);
        let reply = "<spoken>First sentence is here. Second sentence follows right behind. Third closes cleanly.</spoken>";
        assert!(calls.delivery(&persona, "both", "Mack", reply, true, Some(&origin)));
        for _ in 0..200 {
            if lock(&fake.spoken).len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(lock(&fake.spoken).len(), 2, "two sentences in flight");
        if hang_up {
            calls.end(&id).unwrap();
        } else {
            calls.interrupt(&id).unwrap();
        }
        for _ in 0..50 {
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
            while let Ok(event) = rx.try_recv() {
                assert!(
                    !matches!(&event, VoiceEvent::Clip { data, .. } if !data.is_empty()),
                    "no audio after {}: {event:?}",
                    if hang_up { "hang-up" } else { "barge-in" }
                );
            }
        }
        assert_eq!(
            lock(&fake.spoken).len(),
            2,
            "the third is never synthesized"
        );
        if !hang_up {
            calls.end(&id).unwrap();
        }
    }
}

/// Only an Ollama voice is warmed when a Turn by turn call starts: a paid or
/// remote one is never asked to speak before the teammate has said anything.
#[tokio::test]
async fn a_voice_other_than_ollama_is_not_prewarmed() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(lock(&fake.spoken).is_empty(), "{:?}", lock(&fake.spoken));
    calls.end(&id).unwrap();
}
