//! A colleague's ask is answered by the recipient's main conversation: it
//! starts a turn there, or waits behind the one in flight, and the reply of
//! that turn goes back to the asker. The pair's thread keeps both lines. One
//! that names a handoff's thread is answered in that thread instead.
use super::gates::{idle, request, turn};
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

pub(super) fn answer_to(room: &Room, id: &str) -> Option<serde_json::Value> {
    room.tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
}

pub(super) fn questions_heard(agents: &Fake) -> usize {
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
pub(super) fn copied(room: &Room, name: &str) -> crate::log::Log {
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

/// The id of the request a `message_teammate` call accepted.
pub(super) fn requested(sent: &serde_json::Value) -> String {
    sent["requestId"].as_str().unwrap().to_string()
}

/// Ada asks `to`, in the work thread the handoff `thread` names.
async fn ask_in(room: &Arc<Room>, to: &str, thread: &str, message: &str) -> Result<String, String> {
    let sent = TeammateTools::new(room, "ada")
        .call(
            "message_teammate",
            &json!({"to": to, "message": message, "intent": "ask", "thread": thread}),
        )
        .await?;
    Ok(requested(&serde_json::from_str(&sent).unwrap()))
}

/// Ada's handoff to Bob, its result heard by her and its thread closed by it.
pub(super) async fn handed_back(room: &Arc<Room>, agents: &Fake) -> (String, String) {
    let handoff = requested(
        &hand_off(room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    done(room, &handoff).await;
    let thread = work_thread(room, &handoff);
    until("Ada has heard the result and the thread has closed", || {
        agents.prompts().len() == 2 && !room.mid_turn("ada") && !room.work_is_open(&thread)
    })
    .await;
    (handoff, thread)
}

pub(super) fn in_thread(room: &Room, thread: &str) -> Vec<serde_json::Value> {
    room.log.load(&StreamId::Side(thread.to_string()))
}

#[tokio::test]
async fn an_ask_naming_a_thread_is_answered_in_it_and_leaves_it_open() {
    let (room, agents) = setup("ask-in-thread");
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;

    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    done(&room, &id).await;

    assert_eq!(
        request(&room, "ada~bob", &id).thread.as_deref(),
        Some(thread.as_str())
    );
    let stream = in_thread(&room, &thread);
    let question = stream
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id.as_str())
        .expect("the question is in the work thread");
    assert_eq!(stream[question]["cause"]["work"], "Fix the build.");
    assert_eq!(stream[question]["cause"]["threadKey"], "ada~bob");
    assert_eq!(stream[question]["text"], "Which branch is it on?");
    let reply = stream[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("the thread answers it");
    let heard = agents
        .prompts()
        .into_iter()
        .find(|p| p.contains("Which branch is it on?"))
        .expect("the thread's agent heard it");
    assert!(
        heard.contains("Ada asks you about the work “Fix the build.”"),
        "{heard}"
    );

    let answer = answer_to(&room, &id).expect("the answer reaches Ada");
    assert_eq!(answer["cause"]["kind"], "peer");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], reply["text"]);
    until("Ada hears the answer", || {
        answer_to(&room, &id).is_some_and(|v| v["receipt"] == "read")
    })
    .await;
    assert!(
        room.work_is_open(&thread),
        "answering an ask does not close the thread"
    );
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["cause"]["kind"] != "question"),
        "Bob's main conversation never heard it"
    );
    assert!(notices(&room, "ada").is_empty(), "an answer is no report");
    let pair = room.log.load(&StreamId::Pair("ada~bob".into()));
    assert!(pair.iter().any(|v| v["id"] == format!("result:{id}")));
}

#[tokio::test]
async fn an_ask_naming_a_thread_not_its_own_is_refused() {
    let log = scratch("ask-in-thread-refused");
    for id in ["ada", "bob", "carol"] {
        enrol(&log, &persona(id));
    }
    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(Scripted::new(vec![])));
    for (to, from) in [("bob", "ada"), ("carol", "ada"), ("bob", "carol")] {
        room.allow_sender(to, from).unwrap();
    }
    let adas = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    let carols = requested(
        &hand_off(&room, "carol", "bob", "Check the lint.", None)
            .await
            .unwrap(),
    );
    for (to, thread) in [
        ("bob", carols.as_str()),
        ("carol", adas.as_str()),
        ("bob", "nope"),
    ] {
        let refused = ask_in(&room, to, thread, "How is it going?").await;
        assert!(
            refused
                .as_ref()
                .is_err_and(|error| error.contains("is not a handoff you sent")),
            "{to} in {thread}: {refused:?}"
        );
    }
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests.len(),
        1,
        "nothing refused was queued"
    );
    assert!(room.exchange_pair("ada~carol").is_none());
    assert_eq!(room.exchange_pair("bob~carol").unwrap().requests.len(), 1);
}

#[tokio::test]
async fn stop_exchange_stops_the_turn_answering_an_ask_in_a_thread() {
    let (room, agents, gate) = gated(
        "ask-in-thread-stopped",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            turn("late", "Too late.", "end_turn"),
            turn("noted", "Noted.", "end_turn"),
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;
    gate.add_permits(2);
    until("the thread's turn answering the ask is running", || {
        request(&room, "ada~bob", &id).started && questions_heard(&agents) == 1
    })
    .await;
    let cancelled = agents.cancel_count();

    room.stop_exchange("ada", "bob").unwrap();
    assert!(
        agents.cancel_count() > cancelled,
        "the thread's turn is cancelled"
    );

    gate.add_permits(10);
    until("the cancelled turn ends", || idle(&room, &thread)).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        room.work_is_open(&thread),
        "the thread stays open: it was never the ask's to close"
    );
    let stopped = request(&room, "ada~bob", &id);
    assert_eq!(stopped.phase, Phase::Stopped);
    assert!(stopped.failed);
    assert_eq!(stopped.reply, "The person stopped this exchange.");
    assert!(
        answer_to(&room, &id).is_none(),
        "no answer is delivered to Ada"
    );
    assert!(
        room.tape("ada").iter().any(|v| v["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("exchange-ended:"))),
        "Ada is told it stopped"
    );
}

/// A turn answering an ask in a thread that fails without a word tells the
/// asker it was the question's turn, as a main conversation's does.
#[tokio::test]
async fn a_failed_turn_answering_an_ask_in_a_thread_says_it_was_the_questions() {
    let (room, agents, gate) = gated(
        "ask-in-thread-failed",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            vec![Update::Turn {
                stop_reason: "failed".into(),
                usage: None,
            }],
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;
    gate.add_permits(10);
    done(&room, &id).await;
    let failed = request(&room, "ada~bob", &id);
    assert!(failed.failed);
    assert_eq!(
        failed.reply,
        "The turn answering this question ended without a result; inspect before retrying."
    );
}

/// The ask's leases end before the handoff's do, as a revocation's do before
/// its record says so: the ask is stopped, and the handoff's turn, which the
/// ask was waiting behind, goes on to its result.
#[tokio::test]
async fn a_stopped_ask_in_a_thread_leaves_the_handoffs_turn_and_the_thread_alone() {
    let (room, agents, gate) = gated(
        "ask-in-thread-sender-gone",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            turn("noted", "Noted.", "end_turn"),
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    let sent = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Which branch is it on?","intent":"ask","thread":handoff}),
        )
        .await
        .unwrap();
    let id = requested(&serde_json::from_str(&sent).unwrap());
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;

    lock(&room.exchange_leases)
        .get(&id)
        .expect("the ask's leases")
        .0
        .revoke();
    until("the ask is stopped", || {
        request(&room, "ada~bob", &id).phase == Phase::Stopped
    })
    .await;
    assert_eq!(agents.cancel_count(), 0, "no turn is cancelled");
    assert!(room.work_is_open(&thread));
    let running = request(&room, "ada~bob", &handoff);
    assert!(running.phase == Phase::Running && running.started);

    gate.add_permits(20);
    done(&room, &handoff).await;
    let results: Vec<_> = room
        .tape("ada")
        .into_iter()
        .filter(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == handoff.as_str())
        .collect();
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
    assert!(
        room.work_is_open(&thread),
        "the ask's stop did not close it"
    );
    assert!(!request(&room, "ada~bob", &id).started);
    assert!(answer_to(&room, &id).is_none());
    assert_eq!(questions_heard(&agents), 0, "no turn took the ask up");
}

/// The person's own turn is the one in flight, with the ask waiting behind
/// it: stopping the exchange does not cancel their turn.
#[tokio::test]
async fn stop_exchange_leaves_the_persons_turn_in_a_thread_alone() {
    let (room, agents, gate) = gated(
        "ask-in-thread-behind-the-person",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            turn("person", "Done for the person.", "end_turn"),
            turn("noted-1", "Noted.", "end_turn"),
            turn("noted-2", "Noted.", "end_turn"),
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    room.prompt_side(&thread, "the person's work", None)
        .await
        .unwrap();
    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;
    gate.add_permits(2);
    until("the person's turn is running", || {
        agents
            .prompts()
            .iter()
            .any(|p| p.contains("the person's work"))
    })
    .await;
    let cancelled = agents.cancel_count();

    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(agents.cancel_count(), cancelled, "their turn goes on");
    assert!(!idle(&room, &thread));
    assert!(room.work_is_open(&thread));

    gate.add_permits(20);
    until("the person's turn ends", || idle(&room, &thread)).await;
    assert_eq!(request(&room, "ada~bob", &id).phase, Phase::Stopped);
    assert_eq!(questions_heard(&agents), 0, "no turn took the ask up");
    assert!(room.work_is_open(&thread));
}

/// A card the thread raises while it answers an ask waits for nobody: the
/// ask is answered whatever the person does, and when they answer the card
/// the thread hears it as it would any other answer.
#[tokio::test]
async fn a_card_raised_while_an_ask_is_answered_in_a_thread_is_heard_when_answered() {
    let (room, agents, gate) = gated(
        "ask-in-thread-card",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            turn("asked", "I asked which colour.", "end_turn"),
            turn("noted-1", "Noted.", "end_turn"),
            turn("noted-2", "Noted.", "end_turn"),
            turn("blue", "Painting it blue.", "end_turn"),
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;
    gate.add_permits(2);
    until("the thread's turn answering the ask is running", || {
        request(&room, "ada~bob", &id).started && questions_heard(&agents) == 1
    })
    .await;
    TeammateTools::new(&room, "bob")
        .for_work(thread.clone())
        .call("request_human", &json!({"reason": "Which colour?"}))
        .await
        .unwrap();
    assert!(
        request(&room, "ada~bob", &id).human_actions.is_empty(),
        "the ask waits on no card"
    );

    gate.add_permits(20);
    done(&room, &id).await;
    let action = in_thread(&room, &thread)
        .iter()
        .find(|v| v["kind"] == "human_action")
        .expect("the card is in the thread")["actionId"]
        .as_str()
        .unwrap()
        .to_string();
    room.answer_human(
        "bob",
        &action,
        crate::contract::HumanAnswer::Done,
        Some("Blue".into()),
    )
    .unwrap();
    until("the thread hears the person's answer", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["id"] == format!("human-answer:{action}") && v["receipt"] == "read")
    })
    .await;
    assert!(
        agents
            .prompts()
            .iter()
            .any(|p| p.contains("The person answered your request") && p.contains("Blue")),
        "{:?}",
        agents.prompts()
    );
}

/// The handoff's turn is still at work when the ask arrives: the ask waits in
/// the thread behind it, the thread that its result would close stays open
/// for it, and the result is still the handoff's own turn's.
#[tokio::test]
async fn an_ask_sent_while_the_handoff_works_waits_behind_its_turn() {
    let (room, agents, gate) = gated(
        "ask-in-thread-behind-the-handoff",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            turn("branch", "It is on main.", "end_turn"),
            turn("noted-1", "Noted.", "end_turn"),
            turn("noted-2", "Noted.", "end_turn"),
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts().len(), 1, "no turn took the ask up");
    assert!(!request(&room, "ada~bob", &id).started);

    gate.add_permits(20);
    done(&room, &handoff).await;
    done(&room, &id).await;
    let said: Vec<String> = in_thread(&room, &thread)
        .iter()
        .filter(|v| v["kind"] == "agent")
        .map(|v| v["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(said.len(), 2, "{said:?}");
    assert_ne!(said[0], said[1]);
    assert_eq!(request(&room, "ada~bob", &handoff).reply, said[0]);
    assert_eq!(request(&room, "ada~bob", &id).reply, said[1]);
    assert_eq!(answer_to(&room, &handoff).unwrap()["text"], said[0]);
    assert_eq!(answer_to(&room, &id).unwrap()["text"], said[1]);
    assert!(room.work_is_open(&thread), "the ask kept the thread open");
}

#[test]
fn an_ask_is_placed_where_its_handoff_opened_a_thread_and_never_opens_one() {
    let handoff = |phase, thread: Option<&str>| {
        let mut request = saved_request("h", Intent::Handoff, phase);
        request.thread = thread.map(str::to_string);
        request
    };
    let ask = |continues: &str| {
        let mut request = saved_request("ask", Intent::Ask, Phase::Queued);
        request.continues = Some(continues.to_string());
        request
    };
    let pair = |requests| Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: 0,
        paused: false,
        requests,
    };

    let opened = pair(vec![handoff(Phase::Done, Some("t")), ask("h")]);
    assert!(matches!(opened.place(&opened.requests[1]), Ok(Place::In(t)) if t == "t"));
    let to_come = pair(vec![handoff(Phase::Queued, None), ask("h")]);
    assert!(matches!(
        to_come.place(&to_come.requests[1]),
        Ok(Place::Later)
    ));
    let never = pair(vec![handoff(Phase::Reply, None), ask("h")]);
    let refused = never.place(&never.requests[1]).err().unwrap();
    assert!(refused.contains("has not been opened yet"), "{refused}");
    let unknown = pair(vec![ask("nope")]);
    assert!(unknown.place(&unknown.requests[0]).is_err());
}

/// Accepted into the thread, whose close or whose desk's stop kept its line
/// from ever reaching a turn: the ask is Bob's to answer still.
fn lost_ask(room: &Room, handoff: &str, thread: &str) {
    let mut pair = room.exchange_pair("ada~bob").unwrap();
    let mut lost = saved_request("lost", Intent::Ask, Phase::Running);
    lost.message = "Which branch is it on?".into();
    lost.request_counted = true;
    lost.thread = Some(thread.to_string());
    lost.continues = Some(handoff.to_string());
    pair.requests.push(lost);
    room.save_pair(&pair).unwrap();
}

#[tokio::test]
async fn an_ask_that_never_reached_a_turn_in_its_thread_is_delivered_again() {
    let (room, agents) = setup("ask-in-thread-lost");
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;
    lost_ask(&room, &handoff, &thread);
    room.recover_queued_exchanges();
    done(&room, "lost").await;
    let answered = request(&room, "ada~bob", "lost");
    assert!(!answered.failed, "{}", answered.reply);
    assert_eq!(answer_to(&room, "lost").unwrap()["cause"]["status"], "done");
    assert_eq!(questions_heard(&agents), 1);

    // A restart finds it the same way, and a delivery already in the thread
    // is not written twice.
    let (old, old_agents) = setup("ask-in-thread-restart");
    old.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&old, &old_agents).await;
    lost_ask(&old, &handoff, &thread);
    let stream = StreamId::Side(thread.clone());
    let mut link = old
        .log
        .load(&stream)
        .into_iter()
        .find(|v| v["kind"] == "link")
        .unwrap();
    link["state"] = json!("live");
    for ended in ["end", "at", "outcome"] {
        link.as_object_mut().unwrap().remove(ended);
    }
    old.log.append(&stream, &link).unwrap();
    old.log
        .append(&StreamId::Tape("bob".into()), &link)
        .unwrap();
    old.log
        .append(
            &stream,
            &json!({"kind":"delivery","id":"question:lost","ts":1,
                "from":{"thread":"ada~bob","kind":"pair"},
                "cause":{"kind":"question","requestId":"lost","personaId":"ada","name":"Ada",
                    "threadKey":"ada~bob","about":"Which branch is it on?"},
                "text":"Which branch is it on?","receipt":"sent"}),
        )
        .unwrap();
    let log = old.log.clone();
    drop(old);
    let agents = Fake::new(Scripted::new(turn("branch", "On main.", "end_turn")));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_queued_exchanges();
    done(&room, "lost").await;
    assert_eq!(answer_to(&room, "lost").unwrap()["text"], "On main.");
    assert_eq!(questions_heard(&agents), 1, "heard once");
    assert_eq!(
        in_thread(&room, &thread)
            .iter()
            .filter(|v| v["cause"]["requestId"] == "lost")
            .count(),
        1
    );
}

/// The person's cancel of a thread drops what waited behind its turn, an ask
/// too: it fails to its asker at once, and nothing hands it over again.
#[tokio::test]
async fn the_persons_cancel_fails_an_ask_still_waiting_in_a_thread() {
    let (room, agents, gate) = gated(
        "ask-in-thread-cancelled-while-queued",
        vec![
            turn("handed", "Build fixed.", "end_turn"),
            turn("noted", "Noted.", "end_turn"),
        ],
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = requested(
        &hand_off(&room, "ada", "bob", "Fix the build.", None)
            .await
            .unwrap(),
    );
    until("the handoff's turn is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let thread = work_thread(&room, &handoff);
    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    until("the ask waits in the thread", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["cause"]["requestId"] == id.as_str())
    })
    .await;

    room.cancel_side(&thread).unwrap();
    until("Ada is told", || answer_to(&room, &id).is_some()).await;
    let asked = request(&room, "ada~bob", &id);
    assert!(asked.failed && !asked.started);
    let failure = answer_to(&room, &id).unwrap();
    assert_eq!(failure["cause"]["status"], "failed");
    assert!(
        failure["text"]
            .as_str()
            .unwrap()
            .contains("was stopped before answering this question"),
        "{failure}"
    );
    gate.add_permits(20);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(questions_heard(&agents), 0, "no turn took the ask up");
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|v| v["cause"]["requestId"] == id.as_str())
            .count(),
        1
    );
}

/// Bob's first agent starts, and every one after it fails as a harness that
/// went down does.
struct DownAfterFirst {
    agents: Arc<Fake>,
    started: Mutex<bool>,
}
#[async_trait::async_trait]
impl Agents for DownAfterFirst {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        if persona.id == "bob" && std::mem::replace(&mut *lock(&self.started), true) {
            return Err("the harness is down".into());
        }
        self.agents.agent(persona, preamble, said, tools, mcp)
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

#[tokio::test]
async fn a_thread_that_cannot_start_to_answer_fails_the_ask() {
    let log = scratch("ask-in-thread-cannot-start");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let agents = Fake::new(Scripted::new(turn("handed", "Build fixed.", "end_turn")));
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(DownAfterFirst {
            agents: agents.clone(),
            started: Mutex::new(false),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, _) = handed_back(&room, &agents).await;

    let id = ask_in(&room, "bob", &handoff, "Which branch is it on?")
        .await
        .unwrap();
    done(&room, &id).await;
    let failed = request(&room, "ada~bob", &id);
    assert!(failed.failed);
    assert!(
        failed.reply.contains("could not start a thread for this"),
        "{}",
        failed.reply
    );
    assert_eq!(answer_to(&room, &id).unwrap()["cause"]["status"], "failed");
}
