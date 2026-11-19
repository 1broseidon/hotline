//! The desk's acknowledgement while a teammate thinks (`voice.acknowledge`):
//! said in the teammate's voice only when the teammate was told of it and
//! has nothing of its own ready yet, and never when the setting is off.
use super::*;

const LINE: &str = "Looking at the build now.";

fn acknowledging(desk: &crate::desk::Desk, on: bool) {
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({"id": "voice", "value": {"acknowledge": on}}),
            ),
        )
        .unwrap();
}

/// What a call sends within `window`: every line said and every clip.
async fn sent_within(
    rx: &mut broadcast::Receiver<VoiceEvent>,
    window: Duration,
) -> Vec<VoiceEvent> {
    let mut sent = Vec::new();
    let _ = tokio::time::timeout(window, async {
        loop {
            match rx.recv().await {
                Ok(event @ (VoiceEvent::Said { .. } | VoiceEvent::Clip { .. })) => sent.push(event),
                Ok(_) => {}
                Err(_) => return,
            }
        }
    })
    .await;
    sent
}

fn says(events: &[VoiceEvent], line: &str) -> bool {
    events
        .iter()
        .any(|event| matches!(event, VoiceEvent::Said { text, .. } if text == line))
}

/// The turn has gone to the teammate: what the person said is in its hands.
async fn handed_over(calls: &Calls, id: &str) {
    until("the turn goes to the teammate", || {
        calls
            .test_change(id, |call| Ok(call.answering.is_some()))
            .unwrap_or(false)
    })
    .await;
}

fn thread_lines(desk: &crate::desk::Desk, persona: &str) -> Vec<Value> {
    desk.log.load(&StreamId::Call(
        call_link(desk, persona).unwrap()["thread"]
            .as_str()
            .unwrap()
            .into(),
    ))
}

/// A slow teammate: the line is asked for with what the person said, said
/// in the call's voice as one whole clip under a line of its own, kept on
/// the call's thread as the teammate's, and it counts as speech on the turn
/// without being taken for the teammate's first clip.
#[tokio::test]
async fn a_slow_teammate_is_covered_by_one_line_in_its_voice() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    acknowledging(&desk, true);
    *lock(&fake.ack) = Ok(format!("\"{LINE}\""));
    utterance(&calls, &id, 1).unwrap();
    let said = event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == LINE),
    )
    .await;
    let VoiceEvent::Said { id: line, .. } = said else {
        unreachable!()
    };
    let clip = event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Clip { id, .. } if *id == line),
    )
    .await;
    assert!(
        matches!(&clip, VoiceEvent::Clip { index: 0, r#final: true, data, .. } if !data.is_empty()),
        "{clip:?}"
    );
    assert_eq!(
        *lock(&fake.acks),
        [("Mack".to_string(), "Can you check the build?".to_string())]
    );
    assert_eq!(*lock(&fake.spoken), [LINE]);
    let (spoken_at, timing_open) = calls
        .test_change(&id, |call| {
            Ok((call.last_spoken_at, call.first_clip_started.is_some()))
        })
        .unwrap();
    assert!(
        spoken_at.is_some(),
        "the waiting tone and overrun note count from it"
    );
    assert!(
        timing_open,
        "the turn's timing still waits for the teammate"
    );
    until("the line is kept as the teammate's", || {
        thread_lines(&desk, &persona)
            .iter()
            .any(|event| event["kind"] == "agent" && event["text"] == LINE)
    })
    .await;

    // The teammate's own reply follows, and is its first clip.
    let origin = Origin {
        call_id: id.clone(),
        seq: 1,
        direct: true,
        front: false,
    };
    assert!(calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "<spoken>The build is green.</spoken>",
        true,
        Some(&origin)
    ));
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(said.last().unwrap().1, "The build is green.");
    assert!(
        calls
            .test_change(&id, |call| Ok(call.first_clip_started.is_none()))
            .unwrap()
    );
    calls.end(&id).unwrap();
}

/// The teammate's first piece is ready before the line's sound: the line is
/// dropped, and only the teammate is heard.
#[tokio::test]
async fn the_line_is_dropped_when_the_teammate_is_ready_first() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    acknowledging(&desk, true);
    *lock(&fake.ack) = Ok(LINE.into());
    *lock(&fake.delay) = Duration::from_millis(400);
    utterance(&calls, &id, 1).unwrap();
    handed_over(&calls, &id).await;
    let origin = Origin {
        call_id: id.clone(),
        seq: 1,
        direct: true,
        front: false,
    };
    calls.reply_delta(
        &persona,
        "reply-1",
        "<spoken>The build is green.</spoken>",
        &origin,
    );
    let sent = sent_within(&mut rx, Duration::from_millis(1_500)).await;
    assert!(says(&sent, "The build is green."), "{sent:?}");
    assert!(!says(&sent, LINE), "{sent:?}");
    assert!(
        lock(&fake.spoken).iter().any(|spoken| spoken == LINE),
        "it was synthesized"
    );
    calls.end(&id).unwrap();
}

/// An empty line, one too late, the Chat budget refusing it, the Voice
/// budget refusing its sound, a teammate that answered quickly last time,
/// no call assistant, and the setting off: nothing is said for the turn
/// but the teammate's own reply, and no turn ends the call.
#[tokio::test]
async fn nothing_is_said_when_there_is_no_line_to_say_in_time() {
    for case in ["empty", "late", "refused", "quick", "off"] {
        let (_root, desk, calls, id, _persona, mut rx, fake) = direct_call().await;
        acknowledging(&desk, case != "off");
        *lock(&fake.ack) = match case {
            "empty" => Ok("  ".into()),
            "refused" => Err(BUDGET_ERROR.into()),
            _ => Ok(LINE.into()),
        };
        if case == "late" {
            *lock(&fake.ack_delay) = Duration::from_millis(700);
        }
        if case == "quick" {
            calls
                .test_change(&id, |call| {
                    call.last_llm_spoken = Some(Duration::from_millis(1_800));
                    Ok(())
                })
                .unwrap();
        }
        utterance(&calls, &id, 1).unwrap();
        handed_over(&calls, &id).await;
        let sent = sent_within(&mut rx, Duration::from_millis(900)).await;
        assert!(sent.is_empty(), "{case}: {sent:?}");
        assert!(lock(&fake.spoken).is_empty(), "{case}");
        let asked = lock(&fake.acks).len();
        assert_eq!(
            asked,
            usize::from(matches!(case, "empty" | "late" | "refused")),
            "{case}"
        );
        assert!(
            calls
                .test_change(&id, |call| Ok(call.acknowledged.is_none()))
                .unwrap(),
            "{case}: nothing waits for the next prompt"
        );
        calls.end(&id).unwrap();
    }
}

/// A desk with no call assistant has no fast model to write the line.
#[tokio::test]
async fn without_a_call_assistant_nothing_is_said() {
    let (_root, desk, calls, id, _persona, mut rx, fake) = direct_call_with(false).await;
    acknowledging(&desk, true);
    utterance(&calls, &id, 1).unwrap();
    handed_over(&calls, &id).await;
    let sent = sent_within(&mut rx, Duration::from_millis(600)).await;
    assert!(sent.is_empty(), "{sent:?}");
    assert!(lock(&fake.acks).is_empty());
    calls.end(&id).unwrap();
}

/// The Voice budget refuses the line's sound: nothing is said for it, and
/// the call carries on to the teammate's turn rather than ending on it.
#[tokio::test]
async fn a_refused_voice_budget_skips_the_line_without_ending_the_call() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let persona = mack(&desk).await;
    acknowledging(&desk, true);
    spending(&desk, json!({"chat": {}, "voice": {"dayUsd": 0.000_001}}));
    *lock(&fake.ack) = Ok(LINE.into());
    let id = Uuid::new_v4().to_string();
    calls
        .start_with_input(
            &id,
            Some(persona),
            false,
            VoiceInputMode::Text,
            desk.clone(),
        )
        .unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    calls
        .text(&id, 1, "Can you check the build?", None)
        .unwrap();
    handed_over(&calls, &id).await;
    let sent = sent_within(&mut rx, Duration::from_millis(600)).await;
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(lock(&fake.acks).len(), 1);
    assert!(
        lock(&fake.spoken).is_empty(),
        "refused before it was spoken"
    );
    assert!(calls.is_up(&id), "the call goes on");
    calls.end(&id).unwrap();
}

/// Speaking over the call, holding it or hanging up while the line is
/// being synthesized: it is never played.
#[tokio::test]
async fn interrupt_hold_and_hang_up_stop_the_line() {
    for how in ["interrupt", "hold", "hang up"] {
        let (_root, desk, calls, id, _persona, mut rx, fake) = direct_call().await;
        acknowledging(&desk, true);
        *lock(&fake.ack) = Ok(LINE.into());
        *lock(&fake.delay) = Duration::from_millis(300);
        utterance(&calls, &id, 1).unwrap();
        until("the line is being synthesized", || {
            lock(&fake.spoken).iter().any(|spoken| spoken == LINE)
        })
        .await;
        match how {
            "interrupt" => calls.interrupt(&id).unwrap(),
            "hold" => calls.hold(&id, true).unwrap(),
            _ => calls.end(&id).unwrap(),
        }
        let sent = sent_within(&mut rx, Duration::from_millis(700)).await;
        assert!(
            !sent
                .iter()
                .any(|event| matches!(event, VoiceEvent::Clip { data, .. } if !data.is_empty())),
            "{how}: {sent:?}"
        );
        assert!(!says(&sent, LINE), "{how}: {sent:?}");
        let _ = calls.end(&id);
    }
}
