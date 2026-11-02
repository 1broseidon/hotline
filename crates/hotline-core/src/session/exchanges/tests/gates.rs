//! A handoff waits for the answers to the questions its turn asks colleagues,
//! as it waits on a card, and what comes of a question, its answer or its
//! failure, reaches the thread that asked it.
use super::handed_over::Apart;
use super::questions::{request, turn};
use super::*;
use crate::thread::ThreadState;
use tokio::sync::Semaphore;

/// Ada's handoff to Bob, its turn held mid-way at `bob_gate`, in a room where
/// Carol's answers are held at `carol_gate`.
struct HandedOff {
    room: Arc<Room>,
    bob: Arc<Fake>,
    bob_gate: Arc<Semaphore>,
    carol_gate: Arc<Semaphore>,
    handoff: String,
    thread: String,
}

async fn handed_off(name: &str) -> HandedOff {
    let log = scratch(name);
    for id in ["ada", "bob", "carol"] {
        enrol(&log, &persona(id));
    }
    let bob_gate = Arc::new(Semaphore::new(0));
    let carol_gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("asked", "I asked Carol which colour.", "end_turn"),
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

/// Bob asks `to` from inside the work thread `thread`, with its tools.
async fn ask_from(room: &Arc<Room>, thread: &str, to: &str) -> String {
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
    room.link_of(&ThreadId::side(thread))
        .is_some_and(|link| matches!(link.state, ThreadState::Closed(_)))
}

fn idle(room: &Room, thread: &str) -> bool {
    !room.work_thread_working(&ThreadId::side(thread))
}

/// Bob's handoff turn has ended to wait on Carol, who is answering.
async fn waiting_on_carol(handed: &HandedOff, ask: &str) {
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Carol, who is answering", || {
        request(&handed.room, "ada~bob", &handed.handoff).phase() == Phase::WaitingHuman
            && request(&handed.room, "bob~carol", ask).phase() == Phase::Running
            && idle(&handed.room, &handed.thread)
    })
    .await;
}

#[tokio::test]
async fn a_handoff_that_asks_a_colleague_waits_for_the_answer_and_carries_on_with_it() {
    let handed = handed_off("handoff-asks-a-colleague").await;
    let room = &handed.room;
    let ask = ask_from(room, &handed.thread, "carol").await;
    waiting_on_carol(&handed, &ask).await;
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
        request(room, "ada~bob", &handed.handoff).phase(),
        Phase::Running,
        "the answer's turn is the handoff's"
    );
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
        request(room, "ada~bob", &handed.handoff).phase(),
        Phase::Running
    );
    handed.bob_gate.add_permits(2);
    done(room, &handed.handoff).await;
    let results = heard_by_ada(room, &handed.handoff);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
    assert_eq!(results[0]["text"], "Painted it blue.");
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["id"] != format!("exchange-ended:{ask}")),
        "the thread heard it, not Bob's main conversation: {:?}",
        room.tape("bob")
            .iter()
            .filter(|v| v["kind"] == "notice")
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_question_stopped_before_its_answer_carries_the_handoff_on_with_the_failure() {
    let handed = handed_off("question-stopped-before-its-answer").await;
    let ask = ask_from(&handed.room, &handed.thread, "carol").await;
    waiting_on_carol(&handed, &ask).await;
    handed.room.stop_exchange("bob", "carol").unwrap();
    carries_on_with_the_failure(&handed, &ask, "Ada", "The person stopped this exchange.").await;
}

/// Carol is deleted while the handoff waits on her answer, in the order the
/// wire's delete does it: her authority is revoked, then she is gone.
#[tokio::test]
async fn a_question_to_a_deleted_teammate_carries_the_handoff_on_with_the_failure() {
    let handed = handed_off("question-to-a-deleted-teammate").await;
    let room = &handed.room;
    let ask = ask_from(room, &handed.thread, "carol").await;
    waiting_on_carol(&handed, &ask).await;
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

/// The link to the desk the question went to ends before it is answered:
/// the link's record, and with it the teammate's name, goes as it is
/// stopped.
#[tokio::test]
async fn a_question_to_a_desk_that_unlinks_carries_the_handoff_on_with_the_failure() {
    let handed = handed_off("question-to-a-desk-that-unlinks").await;
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
        request(room, "ada~bob", &handed.handoff).phase() == Phase::WaitingHuman
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
