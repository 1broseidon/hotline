use super::*;

/// Words arriving in a thread, as the room broadcasts them.
fn said_in(thread: ThreadId, message_id: &str, text: &str) -> StreamDelta {
    StreamDelta::ThreadDelta {
        thread,
        message_id: message_id.to_string(),
        kind: DeltaKind::Text,
        text: text.to_string(),
    }
}

/// Opens a subscription and takes its acknowledgement and empty snapshot.
async fn subscribed_empty(socket: &mut Socket, id: i64, target: Value) {
    ask(socket, json!({ "id": id, "sub": target })).await;
    assert_eq!(heard(socket).await, json!({ "id": id, "ok": true }));
    assert_eq!(heard(socket).await, json!({ "sub": id, "snapshot": [] }));
}

#[tokio::test]
async fn a_tape_carries_the_deltas_nobody_writes_down() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("ephemeral", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "tape": "ada" })).await;

    // A delta for somebody else's tape is not this subscription's business.
    let _ = deltas.send(said_in(ThreadId::dm("bob"), "m1", "not here"));
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m2", "hel"));
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "sub": 1,
            "ephemeral": { "type": "agent_delta", "personaId": "ada", "messageId": "m2", "text": "hel" }
        }),
        "a client that did not declare threads2 is sent the shape the DM has always had"
    );
    let _ = deltas.send(StreamDelta::ThreadDelta {
        thread: ThreadId::dm("ada"),
        message_id: "m3".to_string(),
        kind: DeltaKind::Thought,
        text: "hm".to_string(),
    });
    assert_eq!(
        heard(&mut socket).await["ephemeral"],
        json!({ "type": "thought_delta", "personaId": "ada", "messageId": "m3", "text": "hm" })
    );
}

#[tokio::test]
async fn a_side_thread_carries_only_its_own_deltas_and_never_the_teammates() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("side-ephemeral", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "side": "s1" })).await;

    // The teammate's main words, and another thread's, are not this one's.
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m1", "main"));
    let _ = deltas.send(said_in(ThreadId::side("s2"), "m2", "other"));
    let _ = deltas.send(said_in(ThreadId::side("s1"), "m3", "hel"));
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "sub": 1,
            "ephemeral": { "type": "side_agent_delta", "sideId": "s1", "messageId": "m3", "text": "hel" }
        })
    );
}

#[tokio::test]
async fn a_tape_subscription_never_hears_a_side_threads_deltas() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("side-not-on-tape", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "tape": "ada" })).await;
    let _ = deltas.send(said_in(ThreadId::side("ada"), "m1", "side"));
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m2", "main"));
    assert_eq!(
        heard(&mut socket).await["ephemeral"]["text"],
        "main",
        "the side thread's words stayed out of the main tape"
    );
}

#[tokio::test]
async fn a_threads2_client_is_sent_one_thread_delta_for_every_kind() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("thread-delta", quiet);

    let mut socket = desk(port).await;
    hello_threads2(&mut socket, 1).await;
    // The same thread is named by `threadId` on any kind; the older targets
    // for a tape, a side thread and a run are sent the new delta too.
    subscribed_empty(
        &mut socket,
        2,
        json!({ "threadId": { "kind": "dm", "key": "ada" } }),
    )
    .await;
    subscribed_empty(
        &mut socket,
        3,
        json!({ "threadId": { "kind": "side", "key": "s1" } }),
    )
    .await;
    subscribed_empty(&mut socket, 4, json!({ "run": "r1" })).await;

    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m1", "main"));
    let _ = deltas.send(said_in(ThreadId::side("s1"), "m2", "side"));
    let _ = deltas.send(StreamDelta::ThreadDelta {
        thread: ThreadId::run("r1"),
        message_id: "m3".to_string(),
        kind: DeltaKind::Thought,
        text: "run".to_string(),
    });
    let mut heard_by = HashMap::new();
    for _ in 0..3 {
        let frame = heard_where(&mut socket, |frame| frame.get("ephemeral").is_some()).await;
        heard_by.insert(frame["sub"].as_i64().unwrap(), frame["ephemeral"].clone());
    }
    assert_eq!(
        heard_by[&2],
        json!({ "type": "thread_delta", "thread": { "kind": "dm", "key": "ada" },
                "messageId": "m1", "kind": "text", "text": "main" })
    );
    assert_eq!(
        heard_by[&3],
        json!({ "type": "thread_delta", "thread": { "kind": "side", "key": "s1" },
                "messageId": "m2", "kind": "text", "text": "side" })
    );
    assert_eq!(
        heard_by[&4],
        json!({ "type": "thread_delta", "thread": { "kind": "run", "key": "r1" },
                "messageId": "m3", "kind": "thought", "text": "run" })
    );
}

#[tokio::test]
async fn an_old_client_is_sent_no_delta_for_a_kind_that_never_had_one() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("run-delta", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "run": "r1" })).await;
    subscribed_empty(&mut socket, 2, json!({ "tape": "ada" })).await;
    let _ = deltas.send(said_in(ThreadId::run("r1"), "m1", "run"));
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m2", "main"));
    let frame = heard_where(&mut socket, |frame| frame.get("ephemeral").is_some()).await;
    assert_eq!(frame["sub"], 2, "the run's words were not sent to {frame}");
}

#[tokio::test]
async fn a_phone_may_run_side_threads_the_way_it_runs_a_conversation() {
    for command in [
        Command::SideStart {
            persona_id: "ada".to_string(),
            text: "x".to_string(),
        },
        Command::SidePrompt {
            side_id: "s".to_string(),
            text: "x".to_string(),
            attachments: None,
        },
        Command::SideCancel {
            side_id: "s".to_string(),
        },
        Command::SideArchive {
            side_id: "s".to_string(),
        },
        Command::SideList {
            persona_id: "ada".to_string(),
        },
        Command::SideAnswerPermission {
            side_id: "s".to_string(),
            request_id: "r".to_string(),
            option_id: "o".to_string(),
        },
    ] {
        assert!(Seat::Phone.permits(&command), "{command:?}");
    }
    assert!(Seat::Phone.permits_sub(&Target::Side("s".to_string())));
}
