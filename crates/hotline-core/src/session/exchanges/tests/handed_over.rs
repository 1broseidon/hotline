//! The person in a thread a teammate handed over: their words, the card the
//! handoff waits on, and what reaches the teammate that handed it over.

use super::*;
use crate::contract::HumanActionStatus;
use crate::thread::ThreadState;

/// Bob's agents, and everyone else's, each from their own script.
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

fn closed(room: &Room, thread: &str) -> bool {
    room.link_of(&ThreadId::side(thread))
        .is_some_and(|link| matches!(link.state, ThreadState::Closed(_)))
}

fn pending(room: &Room, action: &str) -> bool {
    matches!(
        room.human_card("bob", action),
        Some((
            TranscriptEvent::HumanAction {
                status: HumanActionStatus::Pending,
                ..
            },
            _
        ))
    )
}

fn said_in(room: &Room, thread: &str) -> Vec<serde_json::Value> {
    room.log.load(&StreamId::Side(thread.to_string()))
}

/// What reached Ada of the request `id`: its results on her tape.
fn heard_by_ada(room: &Room, id: &str) -> Vec<serde_json::Value> {
    room.tape("ada")
        .into_iter()
        .filter(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
        .collect()
}

/// Typing in the thread of a handoff that waits on a card is answering the
/// card: the words are the note of a "Done", and files cannot ride on it.
#[tokio::test]
async fn the_persons_words_answer_the_card_a_handoff_waits_on() {
    let (room, id, action, gate) = handoff_asking("persons-words-answer-the-card").await;
    gate.add_permits(2);
    until(
        "the handoff waits on its card and bob's turn has ended",
        || {
            room.exchange_pair("ada~bob").unwrap().requests[0].phase == Phase::WaitingHuman
                && !room.mid_turn("bob")
        },
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
    assert!(refused.contains("files cannot ride on it"), "{refused}");
    assert!(pending(&room, &action));

    room.prompt_side(&thread, "Approved, go ahead", None)
        .await
        .unwrap();
    let Some((TranscriptEvent::HumanAction { status, note, .. }, _)) =
        room.human_card("bob", &action)
    else {
        panic!("the card is on the thread")
    };
    assert_eq!(status, HumanActionStatus::Done);
    assert_eq!(note.as_deref(), Some("Approved, go ahead"));
    assert!(
        said_in(&room, &thread).iter().all(|v| v["kind"] != "user"),
        "the words went to the card, not into the thread as a line"
    );
    gate.add_permits(2);
    done(&room, &id).await;
    let results = heard_by_ada(&room, &id);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["cause"]["status"], "done");
}

/// A handoff stopped with its card open leaves the card pending in a thread
/// the person can now continue. Nothing waits on that card any more, so
/// their words are a line to the teammate, not an answer no turn would hear.
#[tokio::test]
async fn words_in_a_continued_thread_are_a_line_once_its_handoff_is_over() {
    let (room, id, action, _gate) = handoff_asking("words-after-a-stopped-handoff").await;
    let thread = work_thread(&room, &id);
    room.stop_exchange("ada", "bob").unwrap();
    until("the thread has closed", || closed(&room, &thread)).await;

    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "Where did you get to?", None)
        .await
        .unwrap();
    assert!(
        said_in(&room, &thread)
            .iter()
            .any(|v| v["kind"] == "user" && v["text"] == "Where did you get to?"),
        "the words are a line in the thread"
    );
    assert!(pending(&room, &action));
}

/// The person's line in a thread handed to Bob is theirs: said while the
/// handoff's turn runs it waits behind it, the turn's reply is the handoff's
/// result, and what Bob answers the person stays in the thread, never
/// reaching Ada, who handed it over.
#[tokio::test]
async fn the_persons_line_in_a_handed_over_thread_never_reaches_its_opener() {
    let log = scratch("persons-line-stays");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let reply = |text: &str| {
        vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: text.into(),
                text: text.into(),
            },
            Update::Turn {
                stop_reason: "end_turn".into(),
                usage: None,
            },
        ]
    };
    // Bob's turns are held at the gate; Ada hears her result on her own.
    let agents = Fake::new(
        Scripted::turns(vec![reply("The fix is in."), reply("Darker it is.")]).gated(gate.clone()),
    );
    let apart = Arc::new(Apart {
        bob: agents.clone(),
        others: Fake::new(Scripted::new(reply("Thanks."))),
    });
    let room = Room::with_agents(log, Arc::new(DeskKeys), apart);
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    until("bob is at work on it", || agents.prompts().len() == 1).await;
    let thread = work_thread(&room, &id);

    room.prompt_side(&thread, "Make it darker", None)
        .await
        .unwrap();
    assert_eq!(agents.prompts().len(), 1, "the line waits behind the turn");
    gate.add_permits(2);
    done(&room, &id).await;
    until("the person's line gets its turn", || {
        agents.prompts().len() == 2
    })
    .await;
    assert!(agents.prompts()[1].contains("Make it darker"));
    assert!(
        !closed(&room, &thread),
        "the thread stays open for the line"
    );

    gate.add_permits(2);
    until("bob has answered the person", || {
        said_in(&room, &thread)
            .iter()
            .any(|v| v["kind"] == "agent" && v["text"] == "Darker it is.")
    })
    .await;
    let results = heard_by_ada(&room, &id);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["text"], "The fix is in.");
    assert!(
        room.tape("ada")
            .iter()
            .all(|v| !v.to_string().contains("Darker it is.")
                && !v.to_string().contains("Make it darker")),
        "nothing the person and Bob said reached Ada"
    );
    assert!(
        said_in(&room, &thread)
            .iter()
            .any(|v| v["kind"] == "user" && v["text"] == "Make it darker"),
        "the person's line is theirs in the thread"
    );
}

/// Whether a handed-over thread's link says its handoff no longer stands: on
/// the thread itself, on Bob's tape, and on Ada's copy.
fn says_ended(room: &Room, thread: &str) -> [bool; 3] {
    let id = format!("link:side:{thread}");
    let on = |tape: Vec<serde_json::Value>, id: &str| {
        tape.iter()
            .rev()
            .find(|v| v["id"] == id)
            .is_some_and(|v| v["handoffEnded"] == true)
    };
    [
        room.link_of(&ThreadId::side(thread))
            .is_some_and(|link| link.handoff_ended),
        on(room.tape("bob"), &id),
        on(room.tape("ada"), &format!("{id}@ada")),
    ]
}

/// A handed-over thread's link says its handoff ended when, and only when,
/// Stop exchange, a revocation or an unlink ends it: not while it stands, a
/// finished handoff the person goes on with included, and no more once a new
/// handoff into it stands.
#[tokio::test]
async fn a_threads_link_says_its_handoff_ended_when_and_only_when_it_does() {
    let (room, _) = setup("link-says-ended");
    room.allow_sender("bob", "ada").unwrap();
    let handoff = send(&room, "handoff").await;
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    assert_eq!(says_ended(&room, &thread), [false; 3], "done, it stands");

    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "and the date?", None)
        .await
        .unwrap();
    until("the person's turn is over", || {
        said_in(&room, &thread)
            .iter()
            .filter(|v| v["kind"] == "agent")
            .count()
            == 2
            && room.sides("bob").iter().all(|side| !side.working)
    })
    .await;
    assert_eq!(
        says_ended(&room, &thread),
        [false; 3],
        "continued, it stands"
    );

    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(
        says_ended(&room, &thread),
        [true; 3],
        "said in the live thread"
    );
    room.park_side_thread(&thread).unwrap();
    assert_eq!(
        says_ended(&room, &thread),
        [true; 3],
        "and kept as it parks"
    );
    let listed = room.handoff_threads_for("ada", "bob");
    assert!(listed.is_empty(), "{listed:?}");

    room.resume_exchange("ada", "bob").unwrap();
    let again = hand_off(&room, "ada", "bob", "Work again.", Some(&handoff))
        .await
        .unwrap();
    let again = again["requestId"].as_str().unwrap().to_string();
    done(&room, &again).await;
    assert_eq!(work_thread(&room, &again), thread);
    assert_eq!(
        says_ended(&room, &thread),
        [false; 3],
        "a new handoff stands"
    );
}

/// A revocation, or the person's Stop exchange on the pair, ends the standing
/// of every handoff between the two, finished or not; an ask has none.
#[test]
fn only_a_handoff_is_released_and_once() {
    let mut handoff = saved_request("handoff", Intent::Handoff, Phase::Done);
    handoff.thread = Some("t1".into());
    let mut ask = saved_request("ask", Intent::Ask, Phase::Done);
    ask.thread = Some("t2".into());
    let mut ended = Vec::new();
    assert!(handoff.release(&mut ended));
    assert!(!handoff.release(&mut ended), "it is released once");
    assert!(!ask.release(&mut ended));
    assert!(handoff.released && !ask.released);
    assert_eq!(ended, ["t1"], "its thread's link is to say so");
    let saved = serde_json::to_value(&ask).unwrap();
    assert!(saved.get("released").is_none(), "{saved}");
}

/// A revocation that touches either teammate ends the standing of a finished
/// handoff between them, and its thread's link says so.
#[tokio::test]
async fn a_revocation_ends_a_finished_handoffs_standing() {
    let (room, _) = setup("revocation-ends-standing");
    room.allow_sender("bob", "ada").unwrap();
    let handoff = send(&room, "handoff").await;
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    room.revoke_exchanges("ada");
    assert_eq!(says_ended(&room, &thread), [true; 3]);
}
