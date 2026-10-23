//! A handoff waits for the answers to the questions its turn asks colleagues,
//! as it waits on a card.
use super::*;
use crate::log::Log;
use crate::thread::ThreadState;

/// Bob's agents and everyone else's, each held at a gate of its own: one
/// shared gate would hand Carol's answer the permits meant for the handoff's
/// turn.
pub(super) struct Apart {
    pub(super) bob: Arc<Fake>,
    pub(super) others: Arc<Fake>,
}
#[async_trait::async_trait]
impl Agents for Apart {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        let agents = if persona.id == "bob" {
            &self.bob
        } else {
            &self.others
        };
        agents.agent(persona, preamble, said, tools, mcp)
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

/// One scripted turn: what the teammate says, and how the turn ends.
pub(super) fn turn(id: &str, text: &str, ends: &str) -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: id.into(),
            text: text.into(),
        },
        Update::Turn {
            stop_reason: ends.into(),
            usage: None,
        },
    ]
}

/// Ada's handoff to Bob, its turn held mid-way at `bob_gate`, in a room where
/// Carol's answers are held at `carol_gate`.
pub(super) struct HandedOff {
    pub(super) room: Arc<Room>,
    pub(super) bob: Arc<Fake>,
    pub(super) bob_gate: Arc<Semaphore>,
    pub(super) carol_gate: Arc<Semaphore>,
    pub(super) handoff: String,
    pub(super) thread: String,
}

pub(super) async fn handed_off(name: &str, first_turn_ends: &str) -> HandedOff {
    let log = scratch(name);
    for id in ["ada", "bob", "carol"] {
        enrol(&log, &persona(id));
    }
    let bob_gate = Arc::new(Semaphore::new(0));
    let carol_gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("asked", "I asked Carol which colour.", first_turn_ends),
            turn("painted", "Painted it blue.", "end_turn"),
        ])
        .gated(bob_gate.clone()),
    );
    let others = Fake::new(
        Scripted::new(turn("blue", "Use the blue one.", "end_turn")).gated(carol_gate.clone()),
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Apart {
            bob: bob.clone(),
            others,
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("carol", "bob").unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    HandedOff {
        room,
        bob,
        bob_gate,
        carol_gate,
        handoff,
        thread,
    }
}

/// Bob asks Carol from inside the work thread `thread`, with its tools.
pub(super) async fn ask_carol(room: &Arc<Room>, thread: &str) -> String {
    ask_from(room, thread, "carol").await
}

pub(super) async fn ask_from(room: &Arc<Room>, thread: &str, to: &str) -> String {
    let sent = TeammateTools::new(room, "bob")
        .for_work(thread.to_string())
        .call(
            "message_teammate",
            &json!({"to":to,"message":"Which colour?","intent":"ask"}),
        )
        .await
        .unwrap();
    serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string()
}

pub(super) fn request(room: &Room, key: &str, id: &str) -> Request {
    room.exchange_pair(key)
        .and_then(|pair| pair.requests.into_iter().find(|r| r.id == id))
        .expect("the request is saved")
}

/// The deliveries on Ada's tape that carry this request.
fn heard_by_ada(room: &Room, id: &str) -> Vec<serde_json::Value> {
    room.tape("ada")
        .into_iter()
        .filter(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
        .collect()
}

fn delivered_in(room: &Room, thread: &str, id: &str) -> Option<serde_json::Value> {
    room.log
        .load(&StreamId::Side(thread.to_string()))
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
}

fn closed(room: &Room, thread: &str) -> bool {
    matches!(
        room.link_of(&ThreadId::side(thread))
            .expect("the thread has a link")
            .state,
        ThreadState::Closed(_)
    )
}

pub(super) fn idle(room: &Room, thread: &str) -> bool {
    !room.work_thread_working(&ThreadId::side(thread))
}

#[tokio::test]
async fn a_handoff_that_asks_a_colleague_waits_for_the_answer_and_carries_on_with_it() {
    let handed = handed_off("handoff-asks-a-colleague", "end_turn").await;
    let room = &handed.room;
    let ask = ask_carol(room, &handed.thread).await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Carol and its turn has ended", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && idle(room, &handed.thread)
    })
    .await;
    assert!(
        heard_by_ada(room, &handed.handoff).is_empty(),
        "waiting is not a result"
    );
    assert!(!closed(room, &handed.thread), "the thread stays open");

    handed.carol_gate.add_permits(2);
    until("the thread takes Carol's answer up", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    assert!(handed.bob.prompts()[1].contains("Use the blue one."));
    let answer = delivered_in(room, &handed.thread, &ask).expect("the answer is in the thread");
    assert_eq!(answer["cause"]["kind"], "peer");
    assert_eq!(
        request(room, "ada~bob", &handed.handoff).phase,
        Phase::Running,
        "the answer's turn is the handoff's"
    );
    assert!(!closed(room, &handed.thread));
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["cause"]["requestId"] != ask.as_str()),
        "the answer never lands in Bob's DM"
    );

    handed.bob_gate.add_permits(2);
    done(room, &handed.handoff).await;
    let results = heard_by_ada(room, &handed.handoff);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
    assert_eq!(results[0]["text"], "Painted it blue.");
}

/// The desk's files as the last process left them, for a new one to start
/// from: the old room holds Carol's turn, so it never lets go of its own.
fn copy_of(room: &Room, name: &str, thread: &str) -> Log {
    let copy = scratch(name);
    for stream in [
        StreamId::Room,
        StreamId::Tape("ada".into()),
        StreamId::Tape("bob".into()),
        StreamId::Tape("carol".into()),
        StreamId::Pair("ada~bob".into()),
        StreamId::Pair("bob~carol".into()),
        StreamId::Side(thread.into()),
    ] {
        for event in room.log.load(&stream) {
            copy.append(&stream, &event).unwrap();
        }
    }
    copy
}

/// Carol's answer saved and handed to the thread, and the desk stopped
/// before the thread heard it.
fn answer_saved(log: &Log, thread: &str, ask: &str) {
    let mut pair: Pair = log
        .load(&StreamId::Room)
        .into_iter()
        .filter(|v| v["kind"] == "exchange_pair" && v["id"] == "bob~carol")
        .find_map(|v| serde_json::from_value(v).ok())
        .unwrap();
    let request = pair.requests.iter_mut().find(|r| r.id == ask).unwrap();
    request.phase = Phase::Done;
    request.reply = "Use the blue one.".into();
    request.reply_counted = true;
    let mut value = serde_json::to_value(&pair).unwrap();
    value["kind"] = json!("exchange_pair");
    log.append(&StreamId::Room, &value).unwrap();
    let delivered = TranscriptEvent::Delivery {
        id: format!("exchange-result:{ask}"),
        ts: 1,
        from: Some(DeliveryFrom::new(
            &ThreadId::pair("bob~carol"),
            Some(ask.to_string()),
        )),
        cause: DeliveryCause::Peer {
            request_id: Some(ask.to_string()),
            persona_id: "carol".into(),
            name: "Ada".into(),
            thread_key: "bob~carol".into(),
            status: PeerStatus::Done,
            about: "Which colour?".into(),
        },
        text: "Use the blue one.".into(),
        receipt: Some(crate::contract::Receipt::Sent),
    };
    log.append(
        &StreamId::Side(thread.into()),
        &serde_json::to_value(delivered).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_handoff_waiting_on_a_colleague_carries_on_after_a_restart() {
    for answered in [false, true] {
        let handed = handed_off(&format!("handoff-asks-restart-{answered}"), "end_turn").await;
        let ask = ask_carol(&handed.room, &handed.thread).await;
        handed.bob_gate.add_permits(2);
        until("the handoff waits on Carol, who is answering", || {
            request(&handed.room, "ada~bob", &handed.handoff).phase == Phase::Waiting
                && request(&handed.room, "bob~carol", &ask).started
                && idle(&handed.room, &handed.thread)
        })
        .await;
        let log = copy_of(
            &handed.room,
            &format!("handoff-asks-restarted-{answered}"),
            &handed.thread,
        );
        if answered {
            answer_saved(&log, &handed.thread, &ask);
        }
        let bob = Fake::new(Scripted::new(turn(
            "finished",
            "Finished with the answer.",
            "end_turn",
        )));
        let others = Fake::new(Scripted::new(turn("heard", "Heard it.", "end_turn")));
        let room = Room::with_agents(
            log,
            Arc::new(DeskKeys),
            Arc::new(Apart {
                bob: bob.clone(),
                others,
            }),
        );
        room.recover_exchanges().await;
        done(&room, &handed.handoff).await;
        let results = heard_by_ada(&room, &handed.handoff);
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0]["text"], "Finished with the answer.");
        let prompts = bob.prompts();
        assert_eq!(prompts.len(), 1, "{prompts:?}");
        assert!(
            !prompts[0].contains("work handoff"),
            "the turn that had started is not run again"
        );
        let heard = if answered {
            "Use the blue one."
        } else {
            "inspect before retrying"
        };
        assert!(prompts[0].contains(heard), "{prompts:?}");
        assert_eq!(
            delivered_in(&room, &handed.thread, &ask).unwrap()["receipt"],
            "read"
        );
        room.recover_exchanges().await;
        assert_eq!(heard_by_ada(&room, &handed.handoff).len(), 1);
        assert_eq!(bob.prompts().len(), 1);
    }
}

#[tokio::test]
async fn a_question_asked_outside_a_handoff_turn_holds_nothing() {
    let handed = handed_off("question-outside-a-handoff", "end_turn").await;
    let room = &handed.room;
    let topic = room.start_side("bob", "topic").await.unwrap().side_id;
    until("the person's thread is at work too", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    let ask = ask_carol(room, &topic).await;
    assert!(
        request(room, "ada~bob", &handed.handoff)
            .human_actions
            .is_empty(),
        "a question from the person's thread holds up no handoff"
    );
    handed.carol_gate.add_permits(2);
    handed.bob_gate.add_permits(100);
    done(room, &handed.handoff).await;
    until("the person's thread has heard Carol", || {
        handed
            .bob
            .prompts()
            .iter()
            .any(|p| p.contains("Use the blue one."))
            && idle(room, &topic)
    })
    .await;
    assert!(delivered_in(room, &topic, &ask).is_some());
    assert_eq!(heard_by_ada(room, &handed.handoff).len(), 1);
    assert!(
        request(room, "ada~bob", &handed.handoff)
            .human_actions
            .is_empty()
    );
    assert_eq!(
        room.link_of(&ThreadId::side(&topic)).unwrap().state,
        ThreadState::Live,
        "the person's thread goes on as it did"
    );
}

#[tokio::test]
async fn the_answer_to_a_delegated_threads_question_reopens_it_and_never_lands_in_the_dm() {
    let (room, agents) = setup("delegated-question-reopens");
    room.allow_sender("bob", "ada").unwrap();
    let handoff = send(&room, "handoff").await;
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    until("the handoff's thread has closed", || closed(&room, &thread)).await;
    let mut asked = saved_request("asked-from-the-thread", Intent::Ask, Phase::Reply);
    asked.from = "bob".into();
    asked.to = "ada".into();
    asked.reply = "Use the blue one.".into();
    asked.reply_thread = Some(thread.clone());
    let mut pair = room.exchange_pair("ada~bob").unwrap();
    pair.requests.push(asked);
    room.save_pair(&pair).unwrap();
    room.wake_exchange("ada~bob");
    done(&room, "asked-from-the-thread").await;
    until("the thread takes the answer up", || {
        agents
            .prompts()
            .iter()
            .any(|p| p.contains("Use the blue one."))
    })
    .await;
    assert!(delivered_in(&room, &thread, "asked-from-the-thread").is_some());
    assert!(!closed(&room, &thread), "the thread came back for it");
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["cause"]["requestId"] != "asked-from-the-thread"),
        "it never lands in Bob's DM"
    );
}

#[tokio::test]
async fn a_question_stopped_before_its_answer_carries_the_handoff_on_with_the_failure() {
    let handed = handed_off("question-stopped-before-its-answer", "end_turn").await;
    let room = &handed.room;
    let ask = ask_carol(room, &handed.thread).await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Carol, who is answering", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && request(room, "bob~carol", &ask).phase == Phase::Running
            && idle(room, &handed.thread)
    })
    .await;
    room.stop_exchange("bob", "carol").unwrap();
    carries_on_with_the_failure(&handed, &ask, "Ada", "The person stopped this exchange.").await;
}

/// The question's failure reaches the thread under the name `name`, the
/// handoff's next turn runs on it, and Ada hears the handoff's result once.
async fn carries_on_with_the_failure(handed: &HandedOff, ask: &str, name: &str, text: &str) {
    let room = &handed.room;
    until("the handoff carries on with the failure", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    let failure = delivered_in(room, &handed.thread, ask).expect("the failure is in the thread");
    assert_eq!(failure["cause"]["kind"], "peer");
    assert_eq!(failure["cause"]["status"], "failed");
    assert_eq!(failure["cause"]["name"], name);
    assert_eq!(failure["text"], text);
    assert!(handed.bob.prompts()[1].contains(text));
    assert_eq!(
        request(room, "ada~bob", &handed.handoff).phase,
        Phase::Running
    );
    handed.bob_gate.add_permits(2);
    done(room, &handed.handoff).await;
    let results = heard_by_ada(room, &handed.handoff);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
    assert_eq!(results[0]["text"], "Painted it blue.");
}

/// The link to the desk the question went to ends before it is answered:
/// the link's record, and with it the teammate's name, goes as it is
/// stopped.
#[tokio::test]
async fn a_question_to_a_desk_that_unlinks_carries_the_handoff_on_with_the_failure() {
    let handed = handed_off("question-to-a-desk-that-unlinks", "end_turn").await;
    let room = &handed.room;
    crate::linked::save(
        &room.log,
        &crate::linked::Record {
            desk_id: "grizzly-desk".into(),
            name: "grizzly".into(),
            dials: true,
            key: None,
            teammates: vec![crate::linked::Known {
                persona_id: "lou".into(),
                name: "Lou".into(),
            }],
        },
    )
    .unwrap();
    let ask = ask_from(room, &handed.thread, "grizzly-desk/lou").await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Lou, whose desk is away", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && idle(room, &handed.thread)
    })
    .await;
    room.unlinked("grizzly-desk").await.unwrap();
    carries_on_with_the_failure(
        &handed,
        &ask,
        "That teammate",
        "Lou is on grizzly, which isn't linked any more.",
    )
    .await;
}

/// Carol is deleted while the handoff waits on her answer, in the order the
/// wire's delete does it: her authority is revoked, then she is gone.
#[tokio::test]
async fn a_question_to_a_deleted_teammate_carries_the_handoff_on_with_the_failure() {
    let handed = handed_off("question-to-a-deleted-teammate", "end_turn").await;
    let room = &handed.room;
    let ask = ask_carol(room, &handed.thread).await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Carol, who is answering", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && request(room, "bob~carol", &ask).phase == Phase::Running
            && idle(room, &handed.thread)
    })
    .await;
    room.invalidate("carol").unwrap();
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "persona", "id": "carol", "deleted": true}),
        )
        .unwrap();
    carries_on_with_the_failure(
        &handed,
        &ask,
        "That teammate",
        "Collaboration was revoked or a participant stopped. Send again if still needed.",
    )
    .await;
}

/// Review Focus 2: an answer that comes back after the handoff waiting on it
/// was stopped, or behind a turn of the handoff that failed, is still heard
/// in the thread, as any delivery is, and returns nothing more to Ada.
#[tokio::test]
async fn an_answer_that_outlives_its_handoff_is_still_heard_in_the_thread() {
    let handed = handed_off("answer-outlives-a-stop", "end_turn").await;
    let room = &handed.room;
    let ask = ask_carol(room, &handed.thread).await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Carol", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && idle(room, &handed.thread)
    })
    .await;
    room.stop_exchange("ada", "bob").unwrap();
    handed.carol_gate.add_permits(2);
    until("the thread takes Carol's answer up", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    assert!(handed.bob.prompts()[1].contains("Use the blue one."));
    assert!(delivered_in(room, &handed.thread, &ask).is_some());
    handed.bob_gate.add_permits(2);
    until("the answer's turn ends", || idle(room, &handed.thread)).await;
    assert_eq!(
        request(room, "ada~bob", &handed.handoff).phase,
        Phase::Stopped
    );
    assert!(heard_by_ada(room, &handed.handoff).is_empty());

    let handed = handed_off("answer-outlives-a-failure", "failed").await;
    let room = &handed.room;
    let ask = ask_carol(room, &handed.thread).await;
    handed.carol_gate.add_permits(2);
    until("Carol's answer waits behind the handoff's turn", || {
        delivered_in(room, &handed.thread, &ask).is_some()
    })
    .await;
    handed.bob_gate.add_permits(2);
    until("the thread takes Carol's answer up", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    assert!(handed.bob.prompts()[1].contains("Use the blue one."));
    handed.bob_gate.add_permits(2);
    done(room, &handed.handoff).await;
    until("the answer's turn ends", || idle(room, &handed.thread)).await;
    let results = heard_by_ada(room, &handed.handoff);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "failed");
}

/// Typing in the thread of a handoff that waits on a card is answering the
/// card: the words are the note of a "Done", and files cannot ride on it.
#[tokio::test]
async fn the_persons_words_answer_the_card_a_handoff_waits_on() {
    let (room, id, action, gate) = handoff_asking("persons-words-answer-the-card").await;
    gate.add_permits(4);
    until(
        "the handoff waits on its card and bob's turn has ended",
        || request(&room, "ada~bob", &id).phase == Phase::Waiting && !room.mid_turn("bob"),
    )
    .await;
    let thread = work_thread(&room, &id);
    let file = crate::contract::Attachment {
        created: None,
        kind: crate::contract::AttachmentKind::File,
        name: "plan.md".into(),
        path: "/tmp/plan.md".into(),
        mime_type: None,
        size: None,
        width: None,
        height: None,
        origin: None,
    };
    let refused = room
        .prompt_side(&thread, "Approved, go ahead", Some(vec![file]))
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        "Answer what it asked you first: your words are its card's note, and files cannot ride on it."
    );
    assert!(matches!(
        room.human_card("bob", &action),
        Some((
            TranscriptEvent::HumanAction {
                status: crate::contract::HumanActionStatus::Pending,
                ..
            },
            _
        ))
    ));

    room.prompt_side(&thread, "Approved, go ahead", None)
        .await
        .unwrap();
    let Some((TranscriptEvent::HumanAction { status, note, .. }, _)) =
        room.human_card("bob", &action)
    else {
        panic!("the card is on the thread")
    };
    assert_eq!(status, crate::contract::HumanActionStatus::Done);
    assert_eq!(note.as_deref(), Some("Approved, go ahead"));
    assert!(
        room.log
            .load(&StreamId::Side(thread))
            .iter()
            .all(|v| v["kind"] != "user"),
        "the words went to the card, not into the thread as a line"
    );
    done(&room, &id).await;
    let results = heard_by_ada(&room, &id);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
}

/// Review Focus 1: a line said while the handoff's own turn runs, with no
/// card open, waits behind it. The turn's reply is the handoff's result,
/// never the reply to the person's line, and the thread stays open for it.
#[tokio::test]
async fn a_line_said_during_a_handoffs_turn_waits_and_is_no_result() {
    let handed = handed_off("line-said-during-a-handoff", "end_turn").await;
    let room = &handed.room;
    room.prompt_side(&handed.thread, "Make it darker", None)
        .await
        .unwrap();
    assert_eq!(
        handed.bob.prompts().len(),
        1,
        "the line waits behind the handoff's turn"
    );

    handed.bob_gate.add_permits(2);
    done(room, &handed.handoff).await;
    until("the person's line gets its turn", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    assert!(handed.bob.prompts()[1].contains("Make it darker"));
    assert!(
        !closed(room, &handed.thread),
        "the thread does not close while that line waits"
    );

    handed.bob_gate.add_permits(2);
    until("the person's turn ends", || idle(room, &handed.thread)).await;
    let results = heard_by_ada(room, &handed.handoff);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
    assert_eq!(
        results[0]["text"], "I asked Carol which colour.",
        "the result is the handoff turn's reply, not the person's turn's"
    );
}

/// A handoff stopped with its card open leaves the card pending in a thread
/// the person can now continue. Nothing waits on that card any more, so
/// their words are a line to the teammate, not an answer no turn would hear.
#[tokio::test]
async fn words_in_a_continued_thread_are_a_line_when_its_handoff_is_over() {
    let (room, id, action, _gate) = handoff_asking("words-after-a-stopped-handoff").await;
    let thread = work_thread(&room, &id);
    room.stop_exchange("ada", "bob").unwrap();
    until("the thread has closed", || closed(&room, &thread)).await;

    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "Where did you get to?", None)
        .await
        .unwrap();
    assert!(
        room.log
            .load(&StreamId::Side(thread))
            .iter()
            .any(|v| v["kind"] == "user" && v["text"] == "Where did you get to?"),
        "the words are a line in the thread"
    );
    assert!(matches!(
        room.human_card("bob", &action),
        Some((
            TranscriptEvent::HumanAction {
                status: crate::contract::HumanActionStatus::Pending,
                ..
            },
            _
        ))
    ));
}
