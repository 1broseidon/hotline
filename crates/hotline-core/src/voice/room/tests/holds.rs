use super::*;

async fn dropped_call() -> (
    tempfile::TempDir,
    Arc<Calls>,
    String,
    broadcast::Receiver<VoiceEvent>,
    CancellationToken,
) {
    let (root, desk, calls) = desk(with_fake(Arc::new(Fake::default())));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut events) = calls.subscribe(&id).unwrap();
    let socket = CancellationToken::new();
    calls
        .bind_connection(
            id.clone(),
            "phone",
            socket.clone(),
            CancellationToken::new(),
        )
        .unwrap();
    socket.cancel();
    event(&mut events, |e| {
        matches!(
            e,
            VoiceEvent::State {
                state: VoiceState::Held,
                reconnecting: true,
                ..
            }
        )
    })
    .await;
    (root, calls, id, events, socket)
}

fn state(calls: &Calls, id: &str) -> (VoiceState, bool) {
    match calls.subscribe(id).unwrap().0 {
        VoiceEvent::State {
            state,
            reconnecting,
            ..
        } => (state, reconnecting),
        other => panic!("{other:?}"),
    }
}

/// Resuming a hold while the transport is down must not wake the call: it
/// is still held, and still reconnecting.
#[tokio::test(start_paused = true)]
async fn releasing_a_hold_during_a_drop_leaves_the_call_held_and_reconnecting() {
    let (_root, calls, id, _events, _socket) = dropped_call().await;
    calls.hold(&id, false).unwrap();
    assert_eq!(state(&calls, &id), (VoiceState::Held, true));
}

/// A hold taken while the transport is down is still there when it returns.
#[tokio::test(start_paused = true)]
async fn a_hold_taken_during_a_drop_survives_the_reconnect() {
    let (_root, calls, id, _events, _socket) = dropped_call().await;
    calls.hold(&id, true).unwrap();
    calls
        .bind_connection(
            id.clone(),
            "phone",
            CancellationToken::new(),
            CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(state(&calls, &id), (VoiceState::Held, false));
    calls.hold(&id, false).unwrap();
    assert_eq!(state(&calls, &id), (VoiceState::Listening, false));
}

/// A hold released during a drop wakes the call when the transport returns.
#[tokio::test(start_paused = true)]
async fn a_hold_released_during_a_drop_wakes_on_the_reconnect() {
    let (_root, calls, id, _events, socket) = dropped_call().await;
    assert!(socket.is_cancelled());
    calls.hold(&id, true).unwrap();
    calls.hold(&id, false).unwrap();
    calls
        .bind_connection(
            id.clone(),
            "phone",
            CancellationToken::new(),
            CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(state(&calls, &id), (VoiceState::Listening, false));
}

/// Ended is terminal wherever the write comes from, and one answer to the
/// teammate is tracked by its oldest unfinished turn.
#[tokio::test]
async fn ended_is_terminal_and_answering_keeps_the_oldest_turn() {
    let (_root, desk, calls) = desk(with_fake(Arc::new(Fake::default())));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    calls
        .test_change(&id, |call| {
            call.begin_answer(1);
            call.begin_answer(2);
            assert_eq!(call.answering, Some(1));
            call.state(VoiceState::Ended, Some(VoiceEndReason::Client));
            call.state(VoiceState::Listening, None);
            assert_eq!(call.state, VoiceState::Ended);
            Ok(())
        })
        .unwrap();
}
