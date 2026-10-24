//! The person's Stop ends the colleagues' work it reaches as their decision:
//! a handoff whose turn it cuts short or whose line it drops before that turn
//! began, and the answer to a question whose turn it cuts short. Each sender
//! is told once, without a wake, and the work never runs again. Their
//! Interrupt only cuts the turn: the work goes on in the next one.
use super::gates::{Apart, ask_carol, handed_off, idle, request, turn};
use super::progress::{asking, claimed, heard_with, room_with, said_in, says_ended};
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

/// The person interrupts Bob's handoff turn to say something instead: the
/// line they had said is heard next, not dropped, its turn's answer is the
/// handoff's result, and the delegation goes on. The cut turn is reported.
#[tokio::test]
async fn the_persons_interrupt_on_a_handoffs_turn_lets_their_line_carry_it_on() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("long", "Running the long command.", "end_turn"),
            turn("short", "Used the short one.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate.clone()),
    );
    let room = room_with("interrupts-a-handoff", &ada, vec![bob.clone()]);
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    gate.add_permits(1);
    until("bob has said what he is doing", || {
        said_in(&room, &thread).len() == 1
    })
    .await;
    room.prompt_side(&thread, "use the short one", None)
        .await
        .unwrap();
    room.interrupt_side(&thread).unwrap();
    until("the person's line is heard", || bob.prompts().len() == 2).await;

    assert!(
        bob.prompts()[1].ends_with("use the short one"),
        "{:?}",
        bob.prompts()
    );
    let going = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (going.phase, going.failed),
        (Phase::Running, false),
        "the handoff goes on"
    );
    assert_eq!(
        notices(&room, "ada"),
        [
            "Ada, in your handoff thread “work handoff”: Running the long command.

This turn was stopped."
        ]
    );
    gate.add_permits(2);
    done(&room, &handoff).await;
    let result = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (result.failed, result.reply.as_str(), result.released),
        (false, "Used the short one.", false)
    );
    let delivered = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == handoff.as_str())
        .expect("the result reaches ada");
    assert_eq!(delivered["cause"]["status"], "done");
    assert_eq!(delivered["text"], "Used the short one.");
    until("the result closes the thread", || {
        !room.work_is_open(&thread)
    })
    .await;
    assert_eq!(
        room.link_of(&ThreadId::side(&thread)).unwrap().state,
        crate::thread::ThreadState::Closed(crate::contract::End::Agent)
    );
    assert_eq!(says_ended(&room, &thread), [false; 3]);
}

/// An interrupt with nothing said leaves the handoff for the person's next
/// line, and the thread parking before one comes ends nothing: the line,
/// said in the parked thread, brings it back and its turn's answer is the
/// handoff's result.
#[tokio::test]
async fn a_handoff_interrupted_with_nothing_said_waits_for_the_next_line() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("long", "Running the long command.", "end_turn"))
            .on_cancel(cancelled())
            .gated(gate),
    );
    // The agent that brings the parked thread back is a new one.
    let back = Fake::new(Scripted::new(turn(
        "short",
        "Used the short one.",
        "end_turn",
    )));
    let room = room_with(
        "interrupts-and-parks",
        &ada,
        vec![bob.clone(), back.clone()],
    );
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    room.interrupt_side(&thread).unwrap();
    until("the cut turn is over", || idle(&room, &thread)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Running);
    assert_eq!(
        bob.prompts().len(),
        1,
        "nothing runs until the person speaks"
    );

    room.park_side_thread(&thread).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let parked = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (parked.phase, parked.failed, parked.released),
        (Phase::Running, false, false),
        "{}",
        parked.reply
    );
    assert!(ada.prompts().is_empty(), "nothing woke ada");

    room.prompt_side(&thread, "use the short one", None)
        .await
        .unwrap();
    done(&room, &handoff).await;
    assert!(
        back.prompts()[0].ends_with("use the short one"),
        "{:?}",
        back.prompts()
    );
    let result = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (result.failed, result.reply.as_str()),
        (false, "Used the short one.")
    );
}

/// A colleague's question that reaches the thread before the person's line
/// is answered on its own; the interrupted handoff still waits for the turn
/// the person's line starts, whose answer is its result.
#[tokio::test]
async fn an_interrupted_handoff_waits_past_a_question_answered_in_its_thread() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("long", "Running the long command.", "end_turn"),
            turn("colour", "Blue.", "end_turn"),
            turn("short", "Used the short one.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate.clone()),
    );
    let room = room_with("interrupts-past-a-question", &ada, vec![bob.clone()]);
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    room.interrupt_side(&thread).unwrap();
    until("the cut turn is over", || idle(&room, &thread)).await;

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
    gate.add_permits(2);
    done(&room, &ask).await;
    assert_eq!(request(&room, "ada~bob", &ask).reply, "Blue.");
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Running);

    room.prompt_side(&thread, "use the short one", None)
        .await
        .unwrap();
    gate.add_permits(2);
    done(&room, &handoff).await;
    let result = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (result.failed, result.reply.as_str()),
        (false, "Used the short one.")
    );
}

/// Ada's question into the thread `handoff` opened, sent with her tools.
async fn ask_into(room: &Arc<Room>, handoff: &str) -> String {
    let sent = TeammateTools::new(room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Which colour?","intent":"ask","thread":handoff}),
        )
        .await
        .unwrap();
    serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A second Interrupt, on the turn answering Ada's question in the thread,
/// keeps the handoff the first one cut: the person's next line carries the
/// handoff to its result, and the line after it answers the question.
#[tokio::test]
async fn a_second_interrupt_keeps_the_first_and_each_next_line_carries_one_on() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("long", "Running the long command.", "end_turn"),
            turn("colour", "Looking at the colours.", "end_turn"),
            turn("short", "Used the short one.", "end_turn"),
            turn("blue", "Blue.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate.clone()),
    );
    let room = room_with("interrupts-twice", &ada, vec![bob.clone()]);
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    room.interrupt_side(&thread).unwrap();
    until("the handoff's cut turn is over", || idle(&room, &thread)).await;
    let ask = ask_into(&room, &handoff).await;
    until("the question's turn is running", || {
        bob.prompts().len() == 2
    })
    .await;
    room.interrupt_side(&thread).unwrap();
    until("the question's cut turn is over", || {
        idle(&room, &thread) && bob.prompts().len() == 2
    })
    .await;
    for id in [&handoff, &ask] {
        assert_eq!(request(&room, "ada~bob", id).phase, Phase::Running);
    }

    room.prompt_side(&thread, "use the short one", None)
        .await
        .unwrap();
    gate.add_permits(2);
    done(&room, &handoff).await;
    assert_eq!(
        request(&room, "ada~bob", &handoff).reply,
        "Used the short one."
    );
    assert!(room.work_is_open(&thread), "the question is still carried");
    room.prompt_side(&thread, "blue", None).await.unwrap();
    gate.add_permits(2);
    done(&room, &ask).await;
    let answer = request(&room, "ada~bob", &ask);
    assert_eq!((answer.failed, answer.reply.as_str()), (false, "Blue."));
}

/// The person's Stop on the turn answering Ada's question in the thread
/// ends that answer, and the handoff an earlier Interrupt left the thread
/// carrying as well: each is told once, nothing wakes Ada, and neither runs
/// again.
#[tokio::test]
async fn the_persons_stop_ends_what_an_interrupt_left_its_thread_carrying() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("long", "Running the long command.", "end_turn"),
            turn("colour", "Looking at the colours.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate.clone()),
    );
    let room = room_with("stops-what-is-carried", &ada, vec![bob.clone()]);
    room.start("ada").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    room.interrupt_side(&thread).unwrap();
    until("the handoff's cut turn is over", || idle(&room, &thread)).await;
    let ask = ask_into(&room, &handoff).await;
    until("the question's turn is running", || {
        bob.prompts().len() == 2
    })
    .await;
    room.cancel_side(&thread).unwrap();
    until("the question's turn is over", || {
        request(&room, "ada~bob", &ask).phase != Phase::Running && idle(&room, &thread)
    })
    .await;

    stopped_for_the_person(&room, &ask, ANSWER_STOPPED);
    stopped_for_the_person(&room, &handoff, HANDOFF_STOPPED);
    let told = notices(&room, "ada");
    for said in [
        format!("Ada, in your handoff thread “work handoff”: {HANDOFF_STOPPED}"),
        format!("Ada, answering “Which colour?”: {ANSWER_STOPPED}"),
    ] {
        assert_eq!(told.iter().filter(|t| **t == said).count(), 1, "{told:?}");
    }
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    room.park_side_thread(&thread).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(bob.prompts().len(), 2, "the stopped work never runs");
    assert!(ada.prompts().is_empty(), "nothing woke ada");
    assert!(nothing_delivered(&room, &handoff) && nothing_delivered(&room, &ask));
}

/// Carol's answer, which would carry Bob's waiting handoff on, waits behind
/// the person's turn in the thread: their Stop on that turn drops it and
/// ends the handoff as theirs, and the thread parking afterwards never hands
/// the answer over again.
#[tokio::test]
async fn the_persons_stop_ends_a_waiting_handoff_whose_answer_it_drops() {
    let handed = handed_off("stops-a-waiting-handoffs-answer", "end_turn").await;
    let room = &handed.room;
    ask_carol(room, &handed.thread).await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Carol", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && idle(room, &handed.thread)
    })
    .await;
    room.prompt_side(&handed.thread, "and the colour?", None)
        .await
        .unwrap();
    until("the person's turn is running", || {
        handed.bob.prompts().len() == 2
    })
    .await;
    handed.carol_gate.add_permits(2);
    until("Carol's answer waits behind the person's turn", || {
        room.log
            .load(&StreamId::Side(handed.thread.clone()))
            .iter()
            .any(|v| v["kind"] == "delivery" && v["cause"]["kind"] == "peer")
    })
    .await;
    room.cancel_side(&handed.thread).unwrap();

    stopped_for_the_person(room, &handed.handoff, HANDOFF_STOPPED);
    assert_eq!(
        notices(room, "ada")[1..],
        [format!(
            "Ada, in your handoff thread “work handoff”: {HANDOFF_STOPPED}"
        )],
        "after the report of the wait"
    );
    until("the person's turn has ended", || idle(room, &handed.thread)).await;
    assert_eq!(says_ended(room, &handed.thread), [true; 3]);
    room.park_side_thread(&handed.thread).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    handed.bob_gate.add_permits(4);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(handed.bob.prompts().len(), 2, "the stopped work never runs");
    stopped_for_the_person(room, &handed.handoff, HANDOFF_STOPPED);
}

/// The person answers the card Bob's handoff waits on in its parked thread,
/// and presses Stop while the thread's agent starts: the answer is dropped,
/// the handoff ends as theirs, and it never runs again.
#[tokio::test]
async fn the_persons_stop_while_a_cards_answer_waits_for_the_agent_ends_its_handoff() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let starts = Arc::new(Semaphore::new(1));
    let bob = Fake::new(
        Scripted::new(turn("waiting", "Waiting for the person.", "end_turn"))
            .gated(gate.clone())
            .slow_to_start(starts.clone()),
    );
    let room = apart("stops-a-cards-answer", &bob, &ada);
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    TeammateTools::new(&room, "bob")
        .for_work(thread.clone())
        .call("request_human", &json!({"reason":"Approve the colour"}))
        .await
        .unwrap();
    gate.add_permits(2);
    until("the handoff waits on the person", || {
        request(&room, "ada~bob", &handoff).phase == Phase::Waiting && idle(&room, &thread)
    })
    .await;
    room.park_side_thread(&thread).unwrap();
    let action = room
        .log
        .load(&StreamId::Side(thread.clone()))
        .iter()
        .find(|v| v["kind"] == "human_action")
        .unwrap()["actionId"]
        .as_str()
        .unwrap()
        .to_string();
    room.answer_human(
        "bob",
        &action,
        crate::contract::HumanAnswer::Done,
        Some("blue".into()),
    )
    .unwrap();
    until("the answer waits for the thread's agent to start", || {
        room.sides
            .get(&thread)
            .is_some_and(|side| side.turn_running())
    })
    .await;
    room.cancel_side(&thread).unwrap();

    stopped_for_the_person(&room, &handoff, HANDOFF_STOPPED);
    starts.add_permits(1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    room.park_side_thread(&thread).unwrap();
    starts.add_permits(4);
    gate.add_permits(4);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(bob.prompts().len(), 1, "the stopped work never runs");
    stopped_for_the_person(&room, &handoff, HANDOFF_STOPPED);
}

/// A restart whose drain runs out cuts the turn of Bob's main conversation
/// answering Ada's question: that is not the person's stop, so the answer
/// ends as one whose turn ended without a result, for Ada to look at before
/// asking again.
#[tokio::test]
async fn a_restart_cutting_a_main_conversations_answer_is_not_the_persons_stop() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("colour", "Looking at the colours.", "end_turn"))
            .on_cancel(cancelled())
            .gated(gate),
    );
    let room = apart("restart-cuts-an-answer", &bob, &ada);
    let ask = send(&room, "ask").await;
    until("bob's main conversation is answering", || {
        bob.prompts().len() == 1
    })
    .await;
    room.stop_for_restart(Duration::from_millis(100)).await;

    let cut = request(&room, "ada~bob", &ask);
    assert_eq!(
        (cut.failed, cut.stopped_by_person, cut.reply.as_str()),
        (
            true,
            false,
            "The turn answering this question ended without a result; inspect before retrying."
        )
    );
    assert_ne!(cut.phase, Phase::Stopped);
    assert!(notices(&room, "ada").is_empty(), "no stop is reported");
}
