//! The person's Stop and Interrupt of a turn doing a colleague's work. Stop
//! ends a handoff for good and tells its sender once, by a note aside that
//! wakes nothing; Interrupt only cuts the turn, and the person's next line
//! carries the work on. An answer either cuts fails to its asker in the
//! person's own words, and only Stop says not to ask again.

use super::handed_over::says_ended;
use super::questions::{answer_to, request, turn};
use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource, End};
use crate::session::peers::{
    ANSWER_INTERRUPTED_BY_THE_PERSON, ANSWER_STOPPED_BY_THE_PERSON, HANDOFF_STOPPED_BY_THE_PERSON,
    STOPPED_BY_THE_PERSON, delivery_wire,
};
use crate::thread::ThreadState;
use tokio::sync::Semaphore;

/// How a scripted turn the person cuts ends.
fn cancelled() -> Vec<Update> {
    vec![Update::Turn {
        stop_reason: "cancelled".into(),
        usage: None,
    }]
}

/// A room where Bob's agents are `bob` and everyone else's `others`, and Ada
/// may hand Bob work.
fn apart(name: &str, bob: &Arc<Fake>, others: &Arc<Fake>) -> Arc<Room> {
    successive(name, vec![bob.clone()], others)
}

/// The same, with each of Bob's agents from the next of `bob`.
fn successive(name: &str, bob: Vec<Arc<Fake>>, others: &Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    for id in ["ada", "bob"] {
        enrol(&log, &persona(id));
    }
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Successive {
            bob: std::sync::Mutex::new(bob),
            others: others.clone(),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    room
}

/// Bob's agents one fake each, in order, the last standing for any after
/// it, so the stop a parked thread's agent is let go of with never cuts the
/// turn of the agent that brings it back; everyone else's from `others`.
struct Successive {
    bob: std::sync::Mutex<Vec<Arc<Fake>>>,
    others: Arc<Fake>,
}
#[async_trait::async_trait]
impl crate::session::Agents for Successive {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        let agents = if persona.id == "bob" {
            let mut bob = lock(&self.bob);
            if bob.len() > 1 {
                bob.remove(0)
            } else {
                bob[0].clone()
            }
        } else {
            self.others.clone()
        };
        agents.agent(persona, preamble, said, tools, mcp)
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

fn said(room: &Room, thread: &str, text: &str) -> bool {
    room.log
        .load(&StreamId::Side(thread.to_string()))
        .iter()
        .any(|v| v["kind"] == "agent" && v["text"] == text)
}

/// The asides Ada was left that say the person stopped her handoff.
fn stopped_notes(room: &Room) -> Vec<Aside> {
    crate::store::asides::of(&room.log, "ada")
        .into_iter()
        .filter(|aside| aside.text.contains(HANDOFF_STOPPED_BY_THE_PERSON))
        .collect()
}

fn thread_open(room: &Room, thread: &str) -> bool {
    !room.work_thread_working(&ThreadId::side(thread)) && room.work_is_open(thread)
}

/// Ada's next line hears `said` once, never as a failure to retry, and the
/// line after does not hear it again.
async fn ada_hears_once(room: &Arc<Room>, ada: &Fake, said: &str) {
    let told = |line: &str| ada.told().into_iter().find(|told| told.contains(line));
    room.prompt("ada", "what happened?", None, None)
        .await
        .unwrap();
    until("ada hears her line", || told("what happened?").is_some()).await;
    let heard = told("what happened?").unwrap();
    assert_eq!(heard.matches(said).count(), 1, "{heard}");
    assert!(!heard.contains("inspect before retrying"), "{heard}");
    until("ada's turn is over", || !room.mid_turn("ada")).await;
    room.prompt("ada", "and now?", None, None).await.unwrap();
    until("ada hears her second line", || told("and now?").is_some()).await;
    assert!(!told("and now?").unwrap().contains(said));
}

/// The person's Stop on the turn of Ada's handoff ends it for good: it is
/// stopped and released, its thread closes as stopped, nothing is delivered
/// and nothing wakes Ada, who is left one note with what Bob had said and
/// the person's word, heard once with her next line. Continued, the thread
/// is the person's, and the stopped work never runs again.
#[tokio::test]
async fn the_persons_stop_on_a_handoffs_turn_ends_it_for_good_and_tells_its_sender_once_by_a_note()
{
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("started", "Running the long command.", "end_turn"))
            .on_cancel(cancelled())
            .gated(gate.clone()),
    );
    let back = Fake::new(Scripted::new(turn("talk", "Here for you.", "end_turn")));
    let room = successive(
        "stops-a-handoffs-turn",
        vec![bob.clone(), back.clone()],
        &ada,
    );
    // Ada is up and idle, so anything that woke her would start a turn.
    room.start("ada").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    gate.add_permits(1);
    until("bob has said what he is doing", || {
        said(&room, &thread, "Running the long command.")
    })
    .await;
    room.cancel_side(&thread).unwrap();
    until("the thread has closed", || !room.work_is_open(&thread)).await;

    let stopped = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (stopped.phase, stopped.failed, stopped.released),
        (Phase::Stopped, true, true)
    );
    assert_eq!(stopped.reply, HANDOFF_STOPPED_BY_THE_PERSON);
    assert_eq!(
        room.link_of(&ThreadId::side(&thread)).unwrap().state,
        ThreadState::Closed(End::Stopped)
    );
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    let notes = stopped_notes(&room);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].level, AsideLevel::Note);
    assert_eq!(notes[0].from, AsideSource::Thread(ThreadId::side(&thread)));
    assert_eq!(
        notes[0].text,
        format!("Running the long command.\n\n{HANDOFF_STOPPED_BY_THE_PERSON}")
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(answer_to(&room, &handoff).is_none(), "no result or failure");
    assert!(
        room.tape("ada").iter().all(|v| v["kind"] != "notice"),
        "nothing in Ada's chat"
    );
    assert!(ada.prompts().is_empty(), "nothing woke ada");
    ada_hears_once(&room, &ada, HANDOFF_STOPPED_BY_THE_PERSON).await;

    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "Where did you get to?", None)
        .await
        .unwrap();
    until("bob answers the person", || {
        said(&room, &thread, "Here for you.")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        back.prompts(),
        ["Where did you get to?"],
        "the stopped work never runs again"
    );
    assert_eq!(bob.prompts().len(), 1);
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Stopped);
    assert!(answer_to(&room, &handoff).is_none());
    assert_eq!(stopped_notes(&room).len(), 1, "told once");
}

/// The person presses Stop while the handoff's thread is still bringing its
/// agent up: the handoff's line waited for it, and it ends there, told to
/// Ada by the person's word alone, and never runs.
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

    let stopped = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (stopped.phase, stopped.started, stopped.released),
        (Phase::Stopped, false, true)
    );
    let notes = stopped_notes(&room);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].text, HANDOFF_STOPPED_BY_THE_PERSON);
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    start.add_permits(1);
    until("the thread's agent is up and idle", || {
        thread_open(&room, &thread)
    })
    .await;
    room.park_side_thread(&thread).unwrap();
    room.wake_exchange("ada~bob");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(bob.prompts().is_empty(), "the stopped work never runs");
    assert!(ada.prompts().is_empty(), "nothing woke ada");
    assert!(answer_to(&room, &handoff).is_none());
}

/// The person's Interrupt only cuts the handoff's turn: the handoff goes on
/// running on its record, nothing ends and nothing wakes Ada, who is left
/// the cut turn's words as any turn's note. Parked meanwhile, the thread
/// keeps it, and the person's next line there carries it to its result.
#[tokio::test]
async fn the_persons_interrupt_only_cuts_a_handoffs_turn_and_their_next_line_carries_it_on() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("red", "Painting it red.", "end_turn"))
            .on_cancel(cancelled())
            .gated(gate.clone()),
    );
    let back = Fake::new(Scripted::new(turn("blue", "Painted it blue.", "end_turn")));
    let room = successive(
        "interrupts-a-handoffs-turn",
        vec![bob.clone(), back.clone()],
        &ada,
    );
    room.start("ada").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    gate.add_permits(1);
    until("bob has said what he is doing", || {
        said(&room, &thread, "Painting it red.")
    })
    .await;
    room.interrupt_side(&thread).unwrap();
    until("the cut turn is over", || thread_open(&room, &thread)).await;

    let carried = request(&room, "ada~bob", &handoff);
    assert_eq!(
        (carried.phase, carried.started, carried.released),
        (Phase::Running, true, false)
    );
    assert_eq!(
        room.carried_into(&thread, None),
        std::slice::from_ref(&handoff)
    );
    assert_eq!(says_ended(&room, &thread), [false; 3]);
    assert!(stopped_notes(&room).is_empty(), "an Interrupt is no stop");
    let notes = crate::store::asides::of(&room.log, "ada");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        (notes[0].level, notes[0].text.as_str()),
        (AsideLevel::Note, "Painting it red.")
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(answer_to(&room, &handoff).is_none());
    assert!(ada.prompts().is_empty(), "nothing woke ada");

    room.park_side_thread(&thread).unwrap();
    room.wake_exchange("ada~bob");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(bob.prompts().len(), 1, "nothing hands it over again");
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Running);

    room.prompt_side(&thread, "Make it blue instead.", None)
        .await
        .unwrap();
    done(&room, &handoff).await;
    assert_eq!(back.prompts(), ["Make it blue instead."]);
    let result = answer_to(&room, &handoff).expect("the result came back");
    assert_eq!(result["cause"]["status"], "done");
    assert_eq!(result["text"], "Painted it blue.");
    until("the result closes the thread", || {
        !room.work_is_open(&thread)
    })
    .await;
    assert_eq!(
        room.link_of(&ThreadId::side(&thread)).unwrap().state,
        ThreadState::Closed(End::Agent)
    );
}

/// After an Interrupt, the person's Stop ends what the thread carries as
/// their stop of it, though no turn is running: Ada is told once by the
/// person's word, the thread goes on, and the person's next line there is
/// only theirs.
#[tokio::test]
async fn the_persons_stop_after_an_interrupt_ends_the_handoff_the_thread_carried() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("red", "Painting it red.", "end_turn"),
            turn("talk", "Here for you.", "end_turn"),
        ])
        .on_cancel(cancelled())
        .gated(gate.clone()),
    );
    let room = apart("stops-what-an-interrupt-left", &bob, &ada);
    room.start("ada").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    room.interrupt_side(&thread).unwrap();
    until("the cut turn is over", || thread_open(&room, &thread)).await;
    room.cancel_side(&thread).unwrap();

    let stopped = request(&room, "ada~bob", &handoff);
    assert_eq!((stopped.phase, stopped.released), (Phase::Stopped, true));
    assert_eq!(room.carried_into(&thread, None), Vec::<String>::new());
    let notes = stopped_notes(&room);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].text, HANDOFF_STOPPED_BY_THE_PERSON);
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    assert!(room.work_is_open(&thread), "the thread goes on");

    room.prompt_side(&thread, "Just tell me.", None)
        .await
        .unwrap();
    gate.add_permits(2);
    until("bob answers the person", || bob.prompts().len() == 2).await;
    until("bob's turn is over", || thread_open(&room, &thread)).await;
    assert_eq!(request(&room, "ada~bob", &handoff).phase, Phase::Stopped);
    assert!(answer_to(&room, &handoff).is_none(), "no result for it");
    assert!(ada.prompts().is_empty(), "nothing woke ada");
}

/// The person's Stop or Interrupt of the turn of Bob's main conversation
/// answering Ada's question fails it to her, as any failed answer reaches
/// its asker, with what Bob had said and the person's word: the Stop says
/// not to ask again, the Interrupt that she may, and neither is followed by
/// "send it again".
#[tokio::test]
async fn the_persons_stop_or_interrupt_of_a_main_conversations_answer_fails_it_in_their_words() {
    for (cut, sentence, never) in [
        (Cut::Stop, ANSWER_STOPPED_BY_THE_PERSON, "Ask it again"),
        (
            Cut::Interrupt,
            ANSWER_INTERRUPTED_BY_THE_PERSON,
            "Do not ask it again",
        ),
    ] {
        let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
        let gate = Arc::new(Semaphore::new(0));
        let bob = Fake::new(
            Scripted::new(turn("colour", "Looking at the colours.", "end_turn"))
                .on_cancel(cancelled())
                .gated(gate.clone()),
        );
        let room = apart(&format!("cuts-an-answer-{cut:?}"), &bob, &ada);
        let ask = send(&room, "ask").await;
        until("bob's main conversation is answering", || {
            bob.prompts().len() == 1 && room.mid_turn("bob")
        })
        .await;
        gate.add_permits(1);
        until("bob has said what he is doing", || {
            room.tape("bob").iter().any(|v| v["kind"] == "agent")
        })
        .await;
        match cut {
            Cut::Stop => room.cancel("bob").unwrap(),
            Cut::Interrupt => room.interrupt("bob").unwrap(),
        }
        done(&room, &ask).await;

        let failure = answer_to(&room, &ask).expect("the asker is told");
        assert_eq!(failure["cause"]["status"], "failed", "{cut:?}");
        assert_eq!(
            failure["text"],
            format!("Looking at the colours.\n\n{sentence}"),
            "{cut:?}"
        );
        let cause: DeliveryCause = serde_json::from_value(failure["cause"].clone()).unwrap();
        let heard = delivery_wire(&cause, failure["text"].as_str().unwrap());
        assert!(heard.ends_with(sentence), "{heard}");
        assert!(!heard.contains("Send it again"), "{heard}");
        assert!(!heard.contains(never), "{heard}");
        assert!(!heard.contains("inspect before retrying"), "{heard}");
    }
}

/// A restart whose drain runs out cuts the turn of Bob's main conversation
/// answering Ada's question: that is not the person's word, so the answer
/// ends as one whose turn ended without a result, to look at before asking
/// again.
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
        bob.prompts().len() == 1 && room.mid_turn("bob")
    })
    .await;
    room.stop_for_restart(Duration::from_millis(100)).await;

    let cut = request(&room, "ada~bob", &ask);
    assert!(cut.failed);
    assert_eq!(
        cut.reply,
        "The turn answering this question ended without a result; inspect before retrying."
    );
}

/// The person's word ends what a failure says, after whatever the cut turn
/// had said, so "send it again" never follows it; any other failure still
/// says it.
#[test]
fn send_it_again_is_never_said_after_the_persons_word() {
    let cause = DeliveryCause::Peer {
        request_id: Some("q".into()),
        persona_id: "bob".into(),
        name: "Bob".into(),
        thread_key: "ada~bob".into(),
        status: PeerStatus::Failed,
        about: "the colour".into(),
    };
    for sentence in [
        STOPPED_BY_THE_PERSON,
        HANDOFF_STOPPED_BY_THE_PERSON,
        ANSWER_STOPPED_BY_THE_PERSON,
        ANSWER_INTERRUPTED_BY_THE_PERSON,
    ] {
        for text in [
            sentence.to_string(),
            format!("Looking at the colours.\n\n{sentence}"),
        ] {
            let heard = delivery_wire(&cause, &text);
            assert!(!heard.contains("Send it again"), "{heard}");
        }
    }
    let failed = delivery_wire(
        &cause,
        "The turn answering this question ended without a result; inspect before retrying.",
    );
    assert!(
        failed.contains("Send it again if you still need it."),
        "{failed}"
    );
}
