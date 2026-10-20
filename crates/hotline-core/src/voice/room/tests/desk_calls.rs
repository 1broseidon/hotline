use super::*;

/// A desk call's reply with a link in it is narrated, not read as written.
#[test]
fn a_link_keeps_a_desk_reply_from_being_said_as_written() {
    assert!(!speech_ready("Here it is: https://ketch.run"));
    assert!(speech_ready("The checks passed."));
}

/// A desk call's answer, streamed in sentences, is one line on the
/// dispatcher's tape too.
#[tokio::test]
async fn a_desk_calls_streamed_answer_is_one_line_on_its_tape() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    event(&mut rx, |e| {
        matches!(e, VoiceEvent::Clip { r#final: true, .. })
    })
    .await;
    let agent = || -> Vec<String> {
        desk.log
            .load(&StreamId::Tape(TAPE_ID.into()))
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .map(|event| event["text"].as_str().unwrap().to_string())
            .collect()
    };
    until("the answer", || !agent().is_empty()).await;
    assert_eq!(agent(), ["The first sentence. The second sentence."]);
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn the_room_sweep_ends_a_call_that_has_gone_quiet_and_leaves_one_that_has_not() {
    let (_root, desk, calls, id, persona, _rx, _fake) = direct_call().await;
    let room = desk.test_room();
    let mut looked_again = std::collections::HashMap::new();
    until("the link", || call_link(&desk, &persona).is_some()).await;

    room.sweep(
        crate::session::now_ms() + crate::thread::QUIET_MS - 60_000,
        &mut looked_again,
    )
    .await;
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Listening,
            reason: None,
            listening: false,
        },
        "not quiet for long enough"
    );

    room.sweep(
        crate::session::now_ms() + crate::thread::QUIET_MS + 60_000,
        &mut looked_again,
    )
    .await;
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Ended,
            reason: Some(VoiceEndReason::Idle),
            listening: false,
        }
    );
    until("the detached thread", || {
        call_link(&desk, &persona).is_some_and(|link| link["state"] == "parked")
    })
    .await;
}

#[tokio::test]
async fn a_desk_call_is_kept_on_the_dispatchers_tape_and_is_no_thread() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    utterance(&calls, &id, 1).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    assert!(
        desk.log.load(&StreamId::Call(id.clone())).is_empty(),
        "no thread, and so no call stream"
    );
    assert_ne!(
        desk.log.load(&StreamId::Tape(TAPE_ID.into())),
        [] as [Value; 0]
    );
    calls.end(&id).unwrap();
}

#[test]
fn a_direct_calls_turn_names_its_call_thread_and_a_desk_calls_does_not() {
    let direct = Origin {
        call_id: Uuid::new_v4().to_string(),
        seq: 3,
        direct: true,
        front: false,
        ended: false,
    };
    let from = direct.from().unwrap();
    assert_eq!(from.kind, crate::contract::ThreadKind::Call);
    assert_eq!(from.thread, direct.call_id);
    assert_eq!(from.request.as_deref(), Some("3"));
    assert_eq!(Origin::from_delivery(&from), Some(direct));
    let desk = Origin {
        direct: false,
        ..Origin {
            call_id: Uuid::new_v4().to_string(),
            seq: 1,
            direct: false,
            front: false,
            ended: false,
        }
    };
    assert_eq!(desk.from(), None, "a desk call is no thread");
    let pair =
        crate::contract::DeliveryFrom::new(&crate::contract::ThreadId::pair("a\u{1f}b"), None);
    assert_eq!(Origin::from_delivery(&pair), None);
}

#[test]
fn only_a_question_or_a_request_is_answered_after_the_call() {
    use hotline_voice::test_support::worth_following_up;
    assert!(worth_following_up("what's left on the release checklist?"));
    assert!(worth_following_up("Is it green?"));
    assert!(worth_following_up(
        "check whether the build passed on my laptop"
    ));
    assert!(!worth_following_up("ok thanks"));
    assert!(!worth_following_up("Okay, thanks, bye!"));
    assert!(!worth_following_up("hold on"));
}
