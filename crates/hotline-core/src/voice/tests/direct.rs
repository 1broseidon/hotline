use super::*;

#[tokio::test]
async fn a_direct_calls_lines_are_kept_on_its_thread_and_found_by_search() {
    let (_root, desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    utterance(&calls, &id, 1).unwrap();
    // This desk has no model for Mack, so the words never reach a session.
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == RETRY_LINE),
    )
    .await;
    let origin = on_turn(&calls, &id, 1);
    assert!(calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "<spoken>What's on your mind?</spoken>\n<written>Shown only.</written>",
        true,
        Some(&origin)
    ));
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "What's on your mind?"),
    )
    .await;
    until("the call's lines", || said_on(&desk, &id).len() == 3).await;

    assert_eq!(
        said_on(&desk, &id),
        [
            ("user".to_string(), "Can you check the build?".to_string()),
            ("agent".to_string(), RETRY_LINE.to_string()),
            ("agent".to_string(), "What's on your mind?".to_string()),
        ]
    );
    // The DM shows the call, once, and the call stream heads with the same line.
    until("the link", || call_link(&desk, &persona).is_some()).await;
    let link = call_link(&desk, &persona).unwrap();
    assert_eq!(link["thread"], id.as_str());
    assert_eq!(link["state"], "live");
    assert_eq!(link["personaId"], persona.as_str());

    // The teammate finds what was said, naming the call.
    let found =
        crate::store::search::search_teammate(desk.log.root(), &persona, "build", None).unwrap();
    assert_eq!(found["hits"][0]["thread"], format!("call:{id}"));
    let heard =
        crate::store::search::search_teammate(desk.log.root(), &persona, "mind", None).unwrap();
    assert_eq!(heard["hits"][0]["thread"], format!("call:{id}"));

    calls.end(&id).unwrap();
    until("the closing link", || {
        call_link(&desk, &persona).is_some_and(|link| link["state"] == "closed")
    })
    .await;
    let link = call_link(&desk, &persona).unwrap();
    assert_eq!(link["end"], "person");
    assert_eq!(link["outcome"], "Hung up");
    assert_eq!(
        desk.log
            .load(&StreamId::Tape(persona.clone()))
            .iter()
            .filter(|event| event["kind"] == "link" && event["threadKind"] == "call")
            .count(),
        1,
        "one line, rewritten as the call goes"
    );

    // A closed call reads back as a thread.
    let thread = crate::thread::ThreadStore::new(&desk.log)
        .load(&crate::contract::ThreadId::call(&id))
        .unwrap();
    assert_eq!(
        thread.state,
        crate::thread::ThreadState::Closed(crate::contract::End::Person)
    );
    assert_eq!(
        thread.participants,
        [
            crate::thread::Participant::Person,
            crate::thread::Participant::Voice(persona.clone())
        ]
    );
    assert!(
        crate::thread::ThreadStore::new(&desk.log)
            .list(&persona)
            .iter()
            .any(|listed| listed.id == thread.id)
    );
}

/// A teammate's reply on a call to it has its spoken version said as it
/// streams, as one reply: one line on screen growing under one id, its audio
/// under that id ending in one empty final clip, and one line on the call's
/// thread. Neither the written version nor a tag reaches speech.
#[tokio::test]
async fn a_streamed_reply_says_its_spoken_version_as_one_line() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
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
    until("the reply", || said_on(&desk, &id).len() == 1).await;
    assert_eq!(
        said_on(&desk, &id),
        [("agent".to_string(), spoken.to_string())]
    );
    let stored = desk.log.load(&StreamId::Call(id.clone()));
    assert!(stored.iter().any(|event| event["id"] == line.as_str()));
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
    let model = calls.writer(&persona);
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
    assert_eq!(replies::Replies::open(desk.log.root()).counts(), counted);
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
            &StreamId::Tape(persona.into()),
            &json!({"kind": "agent", "id": event, "ts": 1, "text": text}),
        )
        .unwrap();
}

/// What was said for a reply, as its agent event keeps it.
fn kept_spoken(desk: &crate::desk::Desk, persona: &str, event: &str) -> Option<String> {
    desk.log
        .load(&StreamId::Tape(persona.into()))
        .into_iter()
        .find(|line| line["id"] == event)
        .and_then(|line| line["spoken"].as_str().map(str::to_string))
}

/// A reply that wrote no spoken version says nothing while it streams, so
/// its opening is never heard before what replaces it. Once it is whole, the
/// call assistant is handed it and the person's last words and says it again
/// to be heard; that is said as the reply's one line, kept on the call's
/// thread and on the reply as what was said, and counted as rewritten. A
/// reply with a spoken version, or one short line such as the line before
/// a tool, never asks the call assistant.
#[tokio::test]
async fn a_reply_without_a_spoken_version_is_rewritten_said_and_kept() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .change(&id, |call| {
            call.words = "Why is the test flaky?".into();
            Ok(())
        })
        .unwrap();
    let rewrite = "It was a race on startup, and it's fixed now. Details are in the chat.";
    *lock(&fake.rewrite) = Ok(rewrite.into());

    assert!(calls.delivery(
        &persona,
        "tagged",
        "Mack",
        "<spoken>It's fixed.</spoken><written>Fixed: see the diff.</written>",
        true,
        Some(&origin)
    ));
    one_reply(&mut rx).await;
    assert!(calls.delivery(
        &persona,
        "line",
        "Mack",
        "Let me check the logs.",
        true,
        Some(&origin)
    ));
    assert_eq!(one_reply(&mut rx).await.0[0].1, "Let me check the logs.");
    assert!(
        lock(&fake.rewrites).is_empty(),
        "only a reply to be read is rewritten"
    );

    let (head, tail) = UNTAGGED.split_at(40);
    calls.reply_delta(&persona, "reply-1", head, &origin);
    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, VoiceEvent::Said { .. } | VoiceEvent::Clip { .. }),
            "nothing is said of it while it streams: {event:?}"
        );
    }
    calls.reply_delta(&persona, "reply-1", tail, &origin);
    assert!(calls.delivery(&persona, "reply-1", "Mack", UNTAGGED, true, Some(&origin)));
    let (said, clips) = one_reply(&mut rx).await;
    assert!(said.iter().all(|(line, _)| *line == said[0].0));
    assert_eq!(said.last().unwrap().1, rewrite);
    assert_eq!(clips.len(), 3, "two sentences and the closing clip");
    assert_eq!(
        *lock(&fake.rewrites),
        [("Why is the test flaky?".to_string(), UNTAGGED.to_string())]
    );
    assert_eq!(
        lock(&fake.spoken)[2..],
        [
            "It was a race on startup, and it's fixed now.",
            "Details are in the chat."
        ]
    );
    // The session writes the reply after handing it over; the rewrite waits
    // for it, and is kept beside it as what was said.
    write_reply(&desk, &persona, "reply-1", UNTAGGED);
    until("the rewrite", || {
        kept_spoken(&desk, &persona, "reply-1").as_deref() == Some(rewrite)
    })
    .await;
    until("the call's thread", || {
        said_on(&desk, &id).last().map(|(_, text)| text.as_str()) == Some(rewrite)
    })
    .await;
    let counts = calls.status().replies.unwrap();
    assert_eq!((counts[0].both, counts[0].rewritten), (1, 1), "{counts:?}");
    assert_eq!(counts[0].untagged, 0);
    calls.end(&id).unwrap();
}

/// While the call assistant writes, the call thinks: the person hears the
/// blip-blip, and the call says so again on its heartbeat, so a phone does
/// not give up on it.
#[tokio::test]
async fn the_call_thinks_while_a_reply_is_rewritten() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    *lock(&fake.rewrite) = Ok("It's fixed. Details are in the chat.".into());
    *lock(&fake.rewrite_delay) = REWRITE_WAIT * 2 / 3;
    assert!(REWRITE_WAIT * 2 / 3 > THINKING_AGAIN);
    assert!(calls.delivery(&persona, "reply-1", "Mack", UNTAGGED, true, Some(&origin)));
    let thinking = |e: &VoiceEvent| {
        matches!(
            e,
            VoiceEvent::State {
                state: VoiceState::Thinking,
                ..
            }
        )
    };
    event(&mut rx, thinking).await;
    event(&mut rx, thinking).await;
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(
        said.last().unwrap().1,
        "It's fixed. Details are in the chat."
    );
    calls.end(&id).unwrap();
}

/// When the call assistant cannot rewrite a reply written to be read (it
/// fails, its budget refuses it, it takes too long, or the desk has none),
/// the call says the reply's opening, up to its first code block, as it
/// always did; the reply is counted as untagged, nothing is kept as said for
/// it, and the call goes on.
#[tokio::test]
async fn a_reply_that_cannot_be_rewritten_is_said_up_to_its_first_code_block() {
    for case in ["failed", "budget", "slow", "no assistant"] {
        let (_root, desk, calls, id, persona, mut rx, fake) =
            direct_call_with(case != "no assistant").await;
        let origin = on_turn(&calls, &id, 1);
        *lock(&fake.rewrite) = match case {
            "failed" => Err("The provider is down.".into()),
            "budget" => Err(BUDGET_ERROR.into()),
            _ => Ok("Too late to say.".into()),
        };
        if case == "slow" {
            *lock(&fake.rewrite_delay) = REWRITE_WAIT * 3;
        }
        write_reply(&desk, &persona, "reply-1", UNTAGGED);
        assert!(calls.delivery(&persona, "reply-1", "Mack", UNTAGGED, true, Some(&origin)));
        let (said, _) = one_reply(&mut rx).await;
        assert_eq!(
            said.last().unwrap().1,
            "Here's the fix for the flaky test:",
            "{case}"
        );
        assert_eq!(
            *lock(&fake.spoken),
            ["Here's the fix for the flaky test:"],
            "{case}"
        );
        assert_eq!(
            lock(&fake.rewrites).len(),
            usize::from(case != "no assistant"),
            "{case}"
        );
        let counts = calls.status().replies.unwrap();
        assert_eq!((counts[0].untagged, counts[0].rewritten), (1, 0), "{case}");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(kept_spoken(&desk, &persona, "reply-1"), None, "{case}");
        // A refused budget ends nothing: the call takes the next words.
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
        utterance(&calls, &id, 2).unwrap();
        calls.end(&id).unwrap();
    }
}

/// Speaking over a teammate stops what it is saying and gives the person the
/// floor, without stopping its turn: what it goes on writing of that reply is
/// not said, and the call takes the person's next words at once.
#[tokio::test]
async fn speaking_over_a_reply_stops_it_and_gives_the_person_the_floor() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .change(&id, |call| {
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(&event, VoiceEvent::Said { text, .. } if text.contains("stale")),
            "{event:?}"
        );
        assert!(!matches!(event, VoiceEvent::Clip { .. }), "{event:?}");
    }
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
        .change(&id, |call| {
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
            state: VoiceState::Thinking,
            reason: None,
            listening: true,
        },
        "still working, and taking what the person says"
    );
    calls
        .change(&id, |call| {
            call.seq = Some(2);
            call.answering = Some(2);
            Ok(())
        })
        .unwrap();
    calls.turn_ended(&persona, &origin);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        calls.change(&id, |call| Ok(call.answering)).unwrap(),
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
        .change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await,
        VoiceEvent::State {
            state: VoiceState::Thinking,
            reason: None,
            listening: true,
        }
    );
    // The wire carries it only when it is so.
    let wire = serde_json::to_value(calls.subscribe(&id).unwrap().0).unwrap();
    assert_eq!(wire["listening"], true);
    let listening = serde_json::to_value(VoiceEvent::State {
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
    let speaking = calls.change(&id, |call| Ok(call.speech.clone())).unwrap();
    utterance(&calls, &id, 2).unwrap();
    assert!(speaking.is_cancelled(), "what the call was saying stops");
    assert!(
        calls
            .change(&id, |call| Ok(call.streams.is_empty()))
            .unwrap()
    );
    // Taking those words, the desk has the floor.
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
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
        .change(&id, |call| {
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
        .change(&id, |call| {
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
/// cannot refuse it; a desk call, which has one, is refused.
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
    assert!(!status.available);
    let id = Uuid::new_v4().to_string();
    calls
        .start_target(&id, Some(persona), false, desk.clone())
        .unwrap();
    assert!(calls.ready_for(&id).is_ok());
    calls.end(&id).unwrap();
}
