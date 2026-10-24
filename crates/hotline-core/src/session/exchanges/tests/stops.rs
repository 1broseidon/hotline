//! The person's Stop ends the colleagues' work it reaches as their decision:
//! a handoff whose turn it cuts short or whose line it drops before that turn
//! began, and the answer to a question whose turn it cuts short. Each sender
//! is told once, without a wake, and the work never runs again.
use super::gates::{Apart, ask_carol, idle, request, turn};
use super::progress::{asking, claimed, heard_with, room_with, says_ended};
use super::*;
use tokio::sync::Semaphore;

const HANDOFF_STOPPED: &str =
    "The person stopped this handoff. Do not send it again unless they ask.";
const ANSWER_STOPPED: &str =
    "The person stopped the answer to this question. Do not ask it again unless they ask.";

/// How a scripted turn the person stops ends.
fn cancelled() -> Vec<Update> {
    vec![Update::Turn {
        stop_reason: "cancelled".into(),
        usage: None,
    }]
}

/// Nothing came back to Ada for `id` as a result or an answer.
fn nothing_delivered(room: &Room, id: &str) -> bool {
    room.tape("ada")
        .iter()
        .all(|v| v["kind"] != "delivery" || v["cause"]["requestId"] != id)
}

/// A room where Bob's agents are `bob` and everyone else's `others`.
fn apart(name: &str, bob: &Arc<Fake>, others: &Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    for id in ["ada", "bob"] {
        enrol(&log, &persona(id));
    }
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Apart {
            bob: bob.clone(),
            others: others.clone(),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    room
}

fn stopped_for_the_person(room: &Room, id: &str, reason: &str) {
    let stopped = request(room, "ada~bob", id);
    assert_eq!(
        (stopped.phase, stopped.failed, stopped.reply.as_str()),
        (Phase::Stopped, true, reason)
    );
    assert!(!stopped.progress_wakes, "nothing wakes its sender");
}

/// Ada's next line hears `said` once, never an invitation to retry, and the
/// line after does not hear it again.
async fn ada_hears_once(room: &Arc<Room>, ada: &Fake, said: &str) {
    room.prompt("ada", "what happened?", None, None)
        .await
        .unwrap();
    until("ada hears her line", || {
        heard_with(ada, "what happened?").is_some()
    })
    .await;
    let heard = heard_with(ada, "what happened?").unwrap();
    assert_eq!(heard.matches(said).count(), 1, "{heard}");
    assert!(!heard.contains("inspect before retrying"), "{heard}");
    room.prompt("ada", "and now?", None, None).await.unwrap();
    until("ada hears her second line", || {
        heard_with(ada, "and now?").is_some()
    })
    .await;
    assert!(!heard_with(ada, "and now?").unwrap().contains(said));
}

/// The person presses Stop while the handoff's thread is still bringing its
/// agent up: the handoff's line waited for it, and it ends there.
#[tokio::test]
async fn the_persons_stop_while_a_handoffs_agent_starts_ends_it_and_it_never_runs() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let start = Arc::new(Semaphore::new(0));
    let bob =
        Fake::new(Scripted::new(turn("done", "Done it.", "end_turn")).slow_to_start(start.clone()));
    let room = apart("stops-while-starting", &bob, &ada);
    room.start("ada").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("the handoff waits for bob's agent to start", || {
        request(&room, "ada~bob", &handoff)
            .thread
            .and_then(|thread| room.sides.get(&thread))
            .is_some_and(|side| side.turn_running())
    })
    .await;
    let thread = work_thread(&room, &handoff);
    room.cancel_side(&thread).unwrap();

    stopped_for_the_person(&room, &handoff, HANDOFF_STOPPED);
    assert_eq!(
        notices(&room, "ada"),
        [format!(
            "Ada, in your handoff thread “work handoff”: {HANDOFF_STOPPED}"
        )]
    );
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    start.add_permits(1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    room.park_side_thread(&thread).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(bob.prompts().is_empty(), "the stopped work never runs");
    assert!(ada.prompts().is_empty(), "nothing woke ada");
    assert!(!claimed(&room));
    ada_hears_once(&room, &ada, HANDOFF_STOPPED).await;
}

/// The person is talking in a thread Ada handed over, and Ada's next handoff
/// into it waits behind the person's turn: their Stop on that turn ends the
/// handoff too.
#[tokio::test]
async fn the_persons_stop_on_their_own_turn_ends_a_handoff_waiting_behind_it() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let handing = Fake::new(Scripted::new(turn("result", "Handed back.", "end_turn")));
    // An agent of its own once the thread is continued: the first one is
    // stopped as the thread closes, and that stop would cut its next turn.
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("talk", "Looking at the colour.", "end_turn"),
            turn("again", "Did it again.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate),
    );
    let room = room_with("stops-behind-the-person", &ada, vec![handing, bob.clone()]);
    let handoff = send(&room, "handoff").await;
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    until(
        "ada has heard the result and bob's thread has closed",
        || ada.prompts().len() == 1 && !claimed(&room) && !room.work_is_open(&thread),
    )
    .await;
    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "and the colour?", None)
        .await
        .unwrap();
    until("the person's turn is running", || bob.prompts().len() == 1).await;
    let again = hand_off(&room, "ada", "bob", "work again", Some(&handoff))
        .await
        .unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("the next handoff waits behind the person's turn", || {
        room.log
            .load(&StreamId::Side(thread.clone()))
            .iter()
            .any(|v| v["id"] == format!("handoff:{again}"))
    })
    .await;
    room.cancel_side(&thread).unwrap();

    stopped_for_the_person(&room, &again, HANDOFF_STOPPED);
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Done);
    assert_eq!(
        notices(&room, "ada"),
        [format!(
            "Ada, in your handoff thread “work handoff”: {HANDOFF_STOPPED}"
        )]
    );
    until("the person's turn has ended", || idle(&room, &thread)).await;
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    room.park_side_thread(&thread).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(bob.prompts().len(), 1, "the stopped work never runs");
    assert_eq!(ada.prompts().len(), 1, "nothing woke ada");
    ada_hears_once(&room, &ada, HANDOFF_STOPPED).await;
}

/// The person stops the turn of Bob's main conversation that is answering
/// Ada's question: Ada hears once that the person stopped the answer, with
/// what Bob had said, and is neither woken nor asked to try again.
#[tokio::test]
async fn the_persons_stop_on_a_main_conversations_answer_ends_it_for_the_asker() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("colour", "Looking at the colours.", "end_turn"))
            .on_cancel(cancelled())
            .gated(gate.clone()),
    );
    let room = apart("stops-answer-in-main", &bob, &ada);
    room.start("ada").await.unwrap();
    let ask = send(&room, "ask").await;
    until("bob's main conversation is answering", || {
        bob.prompts().len() == 1
    })
    .await;
    gate.add_permits(1);
    until("bob has said what he is doing", || {
        room.tape("bob").iter().any(|v| v["kind"] == "agent")
    })
    .await;
    room.cancel("bob").unwrap();
    until("the answer's turn has ended", || {
        request(&room, "ada~bob", &ask).phase != Phase::Running
    })
    .await;

    stopped_for_the_person(&room, &ask, ANSWER_STOPPED);
    assert_eq!(
        notices(&room, "ada"),
        [format!(
            "Ada, answering “work ask”: Looking at the colours.

{ANSWER_STOPPED}"
        )]
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(nothing_delivered(&room, &ask), "no failure is handed back");
    assert!(ada.prompts().is_empty(), "nothing woke ada");
    ada_hears_once(&room, &ada, ANSWER_STOPPED).await;
}

/// The same for a question Ada asked in the thread she handed Bob, answered
/// there while the handoff waits on the person: the stop ends the answer,
/// and the thread and its handoff go on.
#[tokio::test]
async fn the_persons_stop_on_a_threads_answer_ends_it_and_leaves_the_handoff() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("waiting", "Waiting for the person.", "end_turn"),
            turn("colour", "Looking at the colours.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate.clone()),
    );
    let room = room_with("stops-answer-in-thread", &ada, vec![bob.clone()]);
    room.start("ada").await.unwrap();
    let (handoff, thread) = asking(&room, TeammateTools::new(&room, "ada"), &bob, "paint it").await;
    gate.add_permits(2);
    until("the handoff waits on the person", || {
        request(&room, "ada~bob", &handoff).phase == Phase::Waiting && idle(&room, &thread)
    })
    .await;
    until("the wait's report has woken ada, once", || {
        ada.prompts().len() == 1 && !claimed(&room)
    })
    .await;
    let reports = notices(&room, "ada").len();
    let sent = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Which colour?","intent":"ask","thread":handoff}),
        )
        .await
        .unwrap();
    let ask = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("the thread is answering", || bob.prompts().len() == 2).await;
    gate.add_permits(1);
    until("bob has said what he is doing", || {
        room.log
            .load(&StreamId::Side(thread.clone()))
            .iter()
            .any(|v| v["kind"] == "agent" && v["text"] == "Looking at the colours.")
    })
    .await;
    room.cancel_side(&thread).unwrap();
    until("the answer's turn has ended", || {
        request(&room, "ada~bob", &ask).phase != Phase::Running && idle(&room, &thread)
    })
    .await;

    stopped_for_the_person(&room, &ask, ANSWER_STOPPED);
    assert_eq!(
        notices(&room, "ada")[reports..],
        [format!(
            "Ada, answering “Which colour?”: Looking at the colours.

{ANSWER_STOPPED}"
        )]
    );
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Waiting);
    assert!(room.work_is_open(&thread), "the thread goes on");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(nothing_delivered(&room, &ask), "no failure is handed back");
    assert_eq!(ada.prompts().len(), 1, "nothing more woke ada");
    ada_hears_once(&room, &ada, ANSWER_STOPPED).await;
}

/// A handoff waiting on a question whose answer the person stopped goes on
/// with that stop, as it would with an answer, and is not told to ask again.
#[tokio::test]
async fn a_handoff_waiting_on_a_question_whose_answer_the_person_stopped_goes_on() {
    let log = scratch("stops-answer-a-handoff-waits-on");
    for id in ["ada", "bob", "carol"] {
        enrol(&log, &persona(id));
    }
    let bob_gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("asked", "I asked Carol which colour.", "end_turn"),
            turn("painted", "Painted it anyway.", "end_turn"),
        ])
        .gated(bob_gate.clone()),
    );
    let carol_gate = Arc::new(Semaphore::new(0));
    let others = Fake::new(
        Scripted::new(turn("blue", "Looking at the colours.", "end_turn"))
            .on_cancel(cancelled())
            .gated(carol_gate),
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Apart {
            bob: bob.clone(),
            others: others.clone(),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("carol", "bob").unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    let ask = ask_carol(&room, &thread).await;
    bob_gate.add_permits(2);
    until("carol's main conversation is answering", || {
        others.prompts().len() == 1
    })
    .await;
    room.cancel("carol").unwrap();
    until("the answer's turn has ended", || {
        room.exchange_pair("bob~carol")
            .and_then(|pair| pair.requests.into_iter().find(|r| r.id == ask))
            .is_some_and(|r| r.phase != Phase::Running)
    })
    .await;
    let asked = room
        .exchange_pair("bob~carol")
        .and_then(|pair| pair.requests.into_iter().find(|r| r.id == ask))
        .unwrap();
    assert_eq!(
        (asked.phase, asked.reply.as_str()),
        (Phase::Stopped, ANSWER_STOPPED)
    );
    bob_gate.add_permits(2);
    until("bob's handoff goes on with the stop", || {
        bob.prompts().len() == 2
    })
    .await;
    let heard = &bob.prompts()[1];
    assert!(heard.contains(ANSWER_STOPPED), "{heard}");
    assert!(!heard.contains("Send it again"), "{heard}");
    assert!(asked.progress.is_empty(), "its failure told it, once");
}
