//! A colleague's ask is answered by the recipient's main conversation: it
//! starts a turn there, or waits behind the one in flight, and the reply of
//! that turn goes back to the asker. The pair's thread keeps both lines.
use super::gates::{request, turn};
use super::*;
use tokio::sync::Semaphore;

/// Ada and Bob, every agent on one gated script.
fn gated(name: &str, turns: Vec<Vec<Update>>) -> (Arc<Room>, Arc<Fake>, Arc<Semaphore>) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::turns(turns).gated(gate.clone()));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    (room, agents, gate)
}

fn answer_to(room: &Room, id: &str) -> Option<serde_json::Value> {
    room.tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
}

fn questions_heard(agents: &Fake) -> usize {
    agents
        .prompts()
        .iter()
        .filter(|p| p.contains("asks you"))
        .count()
}

#[tokio::test]
async fn an_ask_is_answered_by_the_colleagues_main_conversation() {
    let (room, agents) = setup("ask-main-conversation");
    let id = send(&room, "ask").await;
    done(&room, &id).await;

    let bob = room.tape("bob");
    let question = bob
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id.as_str())
        .expect("the question is in Bob's main conversation");
    assert_eq!(bob[question]["cause"]["threadKey"], "ada~bob");
    assert_eq!(bob[question]["text"], "work ask");
    let reply = bob[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("Bob answers it there");
    assert_eq!(reply["text"], "result-0");
    let heard = &agents.prompts()[0];
    assert!(heard.contains("Ada asks you"), "{heard}");
    assert!(
        heard.contains("Hotline sends that reply back to Ada by itself"),
        "{heard}"
    );

    let answer = answer_to(&room, &id).expect("the answer reaches Ada");
    assert_eq!(answer["cause"]["kind"], "peer");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], "result-0");

    let thread = room.log.load(&StreamId::Pair("ada~bob".into()));
    let asked = thread
        .iter()
        .find(|v| v["id"] == id.as_str())
        .expect("the thread keeps the question");
    assert_eq!(asked["text"], "work ask");
    let answered = thread
        .iter()
        .find(|v| v["id"] == format!("result:{id}"))
        .expect("the thread keeps the answer");
    assert_eq!(answered["text"], "result-0");

    assert!(
        room.session("bob").is_ok(),
        "Bob's main conversation answered"
    );
    assert!(
        lock(&agents.preambles)
            .iter()
            .all(|p| !p.contains("replying privately")),
        "no session of its own answers the ask"
    );
}

#[tokio::test]
async fn an_ask_waits_behind_the_persons_turn_in_the_main_conversation() {
    let (room, agents, gate) = gated(
        "ask-behind-the-person",
        vec![
            turn("person", "Done for the person.", "end_turn"),
            turn("port", "The port is 8080.", "end_turn"),
            turn("heard", "Thanks.", "end_turn"),
        ],
    );
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("the person's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let id = send(&room, "ask").await;
    until("the question waits in Bob's conversation", || {
        room.tape("bob")
            .iter()
            .any(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id.as_str())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts().len(), 1, "no question turn began");
    assert!(!request(&room, "ada~bob", &id).started);

    gate.add_permits(20);
    done(&room, &id).await;
    let answer = answer_to(&room, &id).unwrap();
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], "The port is 8080.");
    assert!(agents.prompts()[1].contains("asks you"));
}

#[tokio::test]
async fn an_ask_to_a_stopped_teammate_starts_its_main_conversation() {
    let (room, _) = setup("ask-starts-main");
    assert!(room.session("bob").is_err());
    let id = send(&room, "ask").await;
    done(&room, &id).await;
    assert!(room.session("bob").is_ok());
    assert_eq!(answer_to(&room, &id).unwrap()["cause"]["status"], "done");
}

#[tokio::test]
async fn stop_exchange_leaves_the_main_conversations_turn_alone_and_fails_the_ask() {
    let (room, agents, gate) = gated("ask-stopped", vec![turn("late", "Too late.", "end_turn")]);
    let id = send(&room, "ask").await;
    until("Bob's question turn is running", || {
        request(&room, "ada~bob", &id).started && room.mid_turn("bob")
    })
    .await;
    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(agents.cancel_count(), 0, "Bob's turn is not cancelled");
    assert!(room.mid_turn("bob"));
    let ended = room
        .tape("ada")
        .into_iter()
        .find(|v| v["id"] == format!("exchange-ended:{id}"))
        .expect("Ada is told");
    assert!(
        ended["text"]
            .as_str()
            .unwrap()
            .contains("The person stopped this exchange."),
        "{ended}"
    );

    gate.add_permits(2);
    until("Bob's turn ends", || !room.mid_turn("bob")).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        answer_to(&room, &id).is_none(),
        "the reply is not delivered"
    );
    let stopped = request(&room, "ada~bob", &id);
    assert_eq!(stopped.phase, Phase::Stopped);
    assert_eq!(stopped.reply, "The person stopped this exchange.");
}

/// The desk's files as a process left them, for a new one to start from.
fn copied(room: &Room, name: &str) -> crate::log::Log {
    let copy = scratch(name);
    for stream in [
        StreamId::Room,
        StreamId::Tape("ada".into()),
        StreamId::Tape("bob".into()),
        StreamId::Pair("ada~bob".into()),
    ] {
        for event in room.log.load(&stream) {
            copy.append(&stream, &event).unwrap();
        }
    }
    copy
}

#[tokio::test]
async fn a_restart_during_the_questions_turn_returns_an_uncertain_failure() {
    let (old, _, _gate) = gated(
        "ask-restart-started",
        vec![turn("never", "Never saved.", "end_turn")],
    );
    let id = send(&old, "ask").await;
    until("Bob's question turn has begun", || {
        request(&old, "ada~bob", &id).started
    })
    .await;
    let log = copied(&old, "ask-restarted-started");
    let agents = Fake::new(Scripted::new(turn("heard", "Noted.", "end_turn")));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_exchanges().await;
    done(&room, &id).await;
    let failure = answer_to(&room, &id).unwrap();
    assert_eq!(failure["cause"]["status"], "failed");
    assert!(
        failure["text"]
            .as_str()
            .unwrap()
            .starts_with("Hotline restarted before the result was saved."),
        "{failure}"
    );
    assert_eq!(questions_heard(&agents), 0, "nothing is replayed");
    room.recover_exchanges().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|v| v["cause"]["requestId"] == id.as_str())
            .count(),
        1
    );

    // Accepted and handed to Bob's conversation, but its turn never began.
    let (room, _) = setup("ask-restart-unstarted");
    let mut accepted = saved_request("accepted", Intent::Ask, Phase::Running);
    accepted.request_counted = true;
    seed(&room, accepted, 1, false);
    room.write_value(
        "bob",
        &json!({"kind":"delivery","id":"question:accepted","ts":1,
            "from":{"thread":"ada~bob","kind":"pair"},
            "cause":{"kind":"question","requestId":"accepted","personaId":"ada","name":"Ada",
                "threadKey":"ada~bob","about":"saved work"},
            "text":"saved work","receipt":"sent"}),
    );
    let log = room.log.clone();
    drop(room);
    let agents = Fake::new(Scripted::new(turn("answer", "Here it is.", "end_turn")));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_exchanges().await;
    done(&room, "accepted").await;
    let answer = answer_to(&room, "accepted").unwrap();
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], "Here it is.");
    assert_eq!(questions_heard(&agents), 1, "delivered once");
}

/// An ask to a teammate quiet past the room's eight hours opens its next
/// chapter, on a fresh session, and is answered there rather than revoked
/// with the session it found.
#[tokio::test]
async fn an_ask_to_a_teammate_gone_quiet_is_answered_in_its_next_chapter() {
    let log = scratch("ask-after-a-long-quiet");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let stale = now_ms() - 10 * 3_600_000;
    for event in [
        json!({"kind": "chapter", "id": "c-bob", "ts": stale, "backendId": "hotline"}),
        json!({"kind": "user", "id": "u1", "ts": stale + 1_000, "text": "did the crane jam?"}),
        json!({"kind": "agent", "id": "a1", "ts": stale + 2_000, "text": "It jammed."}),
    ] {
        log.append(&StreamId::Tape("bob".into()), &event).unwrap();
    }
    let agents = Fake::new(Scripted::new(turn("port", "The port is 8080.", "end_turn")));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("bob").await.unwrap();
    let id = send(&room, "ask").await;
    done(&room, &id).await;
    let answer = answer_to(&room, &id).unwrap();
    assert_eq!(answer["cause"]["status"], "done", "{answer}");
    assert_eq!(answer["text"], "The port is 8080.");
    assert!(
        room.tape("bob")
            .iter()
            .any(|v| v["kind"] == "chapter" && v["id"] == "c-bob" && v["closedBy"] == "idle"),
        "the question opened Bob's next chapter"
    );
}

/// A question's turn that is cancelled or fails says nothing certain: its
/// asker is told the turn ended without a result and to look before trying
/// again, not that it was merely unanswered.
#[tokio::test]
async fn a_cancelled_question_turn_tells_the_asker_to_inspect_before_retrying() {
    let (room, _, gate) = gated(
        "ask-cancelled-turn",
        vec![turn("late", "Too late.", "end_turn")],
    );
    let id = send(&room, "ask").await;
    until("Bob's question turn is running", || {
        request(&room, "ada~bob", &id).started && room.mid_turn("bob")
    })
    .await;
    room.cancel("bob").unwrap();
    done(&room, &id).await;
    let failure = answer_to(&room, &id).unwrap();
    assert_eq!(failure["cause"]["status"], "failed");
    let text = failure["text"].as_str().unwrap();
    assert!(text.contains("without a result"), "{text}");
    assert!(text.contains("inspect before retrying"), "{text}");
    gate.add_permits(10);
}

/// A question stopped, or whose asker's authority is revoked, while it waits
/// behind the person's turn is skipped at the door: no turn answers it, and
/// its asker hears only that it was stopped.
#[tokio::test]
async fn an_ask_stopped_while_its_question_waits_is_never_answered() {
    for revoke in [false, true] {
        let (room, agents, gate) = gated(
            &format!("ask-dropped-while-queued-{revoke}"),
            vec![
                turn("person", "Done for the person.", "end_turn"),
                turn("never", "Never said.", "end_turn"),
            ],
        );
        room.start("bob").await.unwrap();
        room.prompt("bob", "the person's work", None, None)
            .await
            .unwrap();
        until("the person's turn is running", || {
            agents.prompts().len() == 1
        })
        .await;
        let id = send(&room, "ask").await;
        until("the question waits in Bob's conversation", || {
            room.tape("bob")
                .iter()
                .any(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id.as_str())
        })
        .await;
        if revoke {
            room.invalidate("ada").unwrap();
        } else {
            room.stop_exchange("ada", "bob").unwrap();
        }
        assert_eq!(request(&room, "ada~bob", &id).phase, Phase::Stopped);

        gate.add_permits(20);
        until("Bob's queue is empty and idle", || {
            !room.mid_turn("bob") && lock(&room.session("bob").unwrap().turns).waiting.is_empty()
        })
        .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(agents.prompts().len(), 1, "no turn took the question up");
        assert!(!request(&room, "ada~bob", &id).started);
        assert!(
            answer_to(&room, &id).is_none(),
            "nothing came back as an answer"
        );
        assert!(
            room.tape("ada")
                .iter()
                .any(|v| v["id"] == format!("exchange-ended:{id}")),
            "Ada is told it stopped"
        );
    }
}

/// While Bob's main conversation answers a colleague, what the person says
/// to Bob is not steered into that turn, whose whole reply goes to the
/// asker: it waits, and is answered on a turn of its own.
#[tokio::test]
async fn the_persons_line_waits_behind_a_question_turn_and_is_answered_on_its_own() {
    let log = scratch("ask-not-steered");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(
        Scripted::turns(vec![
            turn("port", "The port is 8080.", "end_turn"),
            turn("noon", "It is noon.", "end_turn"),
        ])
        .gated(gate.clone())
        .steerable(),
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    let id = send(&room, "ask").await;
    until("Bob's question turn is running", || {
        request(&room, "ada~bob", &id).started && agents.prompts().len() == 1
    })
    .await;
    room.prompt("bob", "what time is it?", None, None)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.steered(), [] as [String; 0], "not steered in");
    assert!(
        lock(&room.session("bob").unwrap().turns)
            .waiting
            .iter()
            .any(|line| line.text.contains("what time is it?")),
        "the person's line waits behind the question"
    );

    gate.add_permits(20);
    done(&room, &id).await;
    assert_eq!(answer_to(&room, &id).unwrap()["text"], "The port is 8080.");
    until("the person's line gets a turn of its own", || {
        agents
            .prompts()
            .iter()
            .any(|p| p.contains("what time is it?") && !p.contains("asks you"))
    })
    .await;
}

/// The person's cancel drops what waited behind Bob's turn, a colleague's
/// question too: it fails to its asker at once, once, and no restart hands
/// it over again.
#[tokio::test]
async fn the_persons_cancel_fails_a_question_still_waiting_behind_the_turn() {
    let (room, agents, gate) = gated(
        "ask-cancelled-while-queued",
        vec![
            turn("person", "Done for the person.", "end_turn"),
            turn("heard", "Noted.", "end_turn"),
        ],
    );
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("the person's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let id = send(&room, "ask").await;
    until("the question waits in Bob's conversation", || {
        room.tape("bob")
            .iter()
            .any(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id.as_str())
    })
    .await;

    room.cancel("bob").unwrap();
    done(&room, &id).await;
    let asked = request(&room, "ada~bob", &id);
    assert!(asked.failed && !asked.started);
    let name = room.persona("bob").unwrap().name;
    let failure = answer_to(&room, &id).expect("Ada is told at once");
    assert_eq!(failure["cause"]["status"], "failed");
    assert_eq!(
        failure["text"],
        format!(
            "{name} was stopped before answering this question. Send it again if you still need it."
        )
    );
    gate.add_permits(20);
    until("Ada has heard it", || {
        answer_to(&room, &id).is_some_and(|v| v["receipt"] == "read")
    })
    .await;
    assert_eq!(questions_heard(&agents), 0, "no turn took the question up");

    let agents = Fake::new(Scripted::new(turn("again", "Never said.", "end_turn")));
    let restarted = Room::with_agents(
        copied(&room, "ask-cancelled-while-queued-restarted"),
        Arc::new(DeskKeys),
        agents.clone(),
    );
    restarted.recover_exchanges().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(questions_heard(&agents), 0, "nothing is handed over again");
    assert_eq!(
        restarted
            .tape("ada")
            .iter()
            .filter(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id.as_str())
            .count(),
        1
    );
}
