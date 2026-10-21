use super::*;

/// A desk call's reply with a link in it is narrated, not read as written.
#[test]
fn a_link_keeps_a_desk_reply_from_being_said_as_written() {
    assert!(!speech_ready("Here it is: https://ketch.run"));
    assert!(speech_ready("The checks passed."));
}

/// A desk call's answer, streamed in sentences, is one line on the
/// call transcript too.
#[tokio::test]
async fn a_desk_calls_streamed_answer_is_one_line_on_its_call() {
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
            .load(&StreamId::Call(id.clone()))
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
            live_error: None,
            reconnecting: false,
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
            live_error: None,
            reconnecting: false,
            state: VoiceState::Ended,
            reason: Some(VoiceEndReason::Idle),
            listening: false,
        }
    );
    until("the disconnect is recorded", || {
        desk.log
            .load(&StreamId::Call(id.clone()))
            .iter()
            .any(|event| {
                event["kind"] == "notice"
                    && event["text"] == "Voice disconnected. This thread stays open."
            })
    })
    .await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(121)).await;
    tokio::time::resume();
    until("the detached thread", || {
        call_link(&desk, &persona).is_some_and(|link| link["state"] == "parked")
    })
    .await;
}

#[tokio::test]
async fn a_no_lead_desk_call_keeps_its_own_transcript() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    utterance(&calls, &id, 1).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    assert!(
        desk.log
            .load(&StreamId::Tape("voice-dispatcher".into()))
            .is_empty(),
        "no synthetic dispatcher tape"
    );
    assert_ne!(desk.log.load(&StreamId::Call(id.clone())), [] as [Value; 0]);
    calls.end(&id).unwrap();
}

#[test]
fn a_direct_calls_turn_names_its_call_thread_and_a_desk_calls_does_not() {
    let direct = Origin {
        call_id: Uuid::new_v4().to_string(),
        seq: 3,
        direct: true,
        front: false,
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
        }
    };
    assert_eq!(desk.from(), None, "a desk call is no thread");
    let pair =
        crate::contract::DeliveryFrom::new(&crate::contract::ThreadId::pair("a\u{1f}b"), None);
    assert_eq!(Origin::from_delivery(&pair), None);
}

#[tokio::test]
async fn hanging_up_removes_the_leads_handoff_tool_from_an_existing_handle() {
    let (_root, desk, calls) = desk(services());
    let persona_id = mack(&desk).await;
    let persona = crate::room::roster(&desk.log)
        .into_iter()
        .find(|p| p.id == persona_id)
        .unwrap();
    let call = Uuid::new_v4().to_string();
    calls.start(&call, desk.clone()).unwrap();
    let room = desk.test_room();
    let thread = crate::contract::ThreadId::call(&call);
    let reach = room.front_reach(&persona, &thread).unwrap();
    assert!(room.lead_call(&persona_id, &thread));
    calls.end(&call).unwrap();
    let error = reach
        .tools
        .call(
            "message_teammate",
            &json!({"to":"anyone","message":"work","intent":"handoff"}),
        )
        .await
        .unwrap_err();
    assert!(error.contains("no tool"), "{error}");
    assert!(!room.lead_call(&persona_id, &thread));
}
