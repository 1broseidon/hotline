//! What a handed-over thread tells the teammate that handed it over: a note
//! for each turn that is not its result, in the colleague's words, and a
//! nudge beside the result's delivery.

use super::handed_over::Apart;
use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};
use tokio::sync::Semaphore;

fn asides(room: &Room, persona_id: &str) -> Vec<Aside> {
    crate::store::asides::of(&room.log, persona_id)
}

#[tokio::test]
async fn a_turn_that_waits_on_the_person_leaves_its_opener_a_note_and_no_message() {
    let (room, id, _action) = suspended_handoff("aside-note-waiting").await;
    let thread = work_thread(&room, &id);
    let notes = asides(&room, "ada");
    assert_eq!(notes.len(), 1, "{notes:?}");
    let note = &notes[0];
    assert_eq!(note.level, AsideLevel::Note);
    assert_eq!(note.by.as_deref(), Some("bob"));
    assert_eq!(note.from, AsideSource::Thread(ThreadId::side(&thread)));
    assert_eq!(note.about, None, "heard in Ada's main conversation");
    assert_eq!(
        note.text,
        "Waiting for the person\n\nWaiting on the person: Approve the deployment"
    );
    assert!(
        room.tape("ada")
            .iter()
            .all(|v| !v.to_string().contains("Waiting for the person")),
        "nothing reaches Ada's chat: {:?}",
        room.tape("ada")
    );
    assert!(
        room.log
            .load(&StreamId::Side(thread))
            .iter()
            .any(|v| v["kind"] == "aside" && v["id"] == note.id.as_str()),
        "the thread's record shows what it told Ada"
    );
}

#[tokio::test]
async fn a_result_comes_back_as_its_delivery_with_a_nudge_heard_once_beside_it() {
    let (room, agents) = setup("aside-result-nudge");
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    done(&room, &id).await;
    let thread = work_thread(&room, &id);
    let delivery = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
        .expect("the result is still a delivery");
    assert_eq!(delivery["text"], "result-0");
    let nudge = asides(&room, "ada")
        .into_iter()
        .find(|aside| aside.id == format!("result:{id}"))
        .expect("the result left a nudge");
    assert_eq!(nudge.level, AsideLevel::Nudge);
    assert_eq!(nudge.text, "result-0");
    assert_eq!(nudge.from, AsideSource::Thread(ThreadId::side(&thread)));
    // The delivery wakes Ada, and its turn hears the nudge as the result
    // itself, not a second time.
    until("ada hears the result", || {
        asides(&room, "ada")
            .iter()
            .all(|aside| aside.heard.is_some())
    })
    .await;
    let told = agents
        .told()
        .into_iter()
        .find(|prompt| prompt.contains("result-0"))
        .expect("ada's turn was told the result");
    assert!(!told.contains("Asides"), "{told}");
    assert_eq!(
        asides(&room, "ada").len(),
        1,
        "no note for the result's own turn"
    );
}

/// The person talks in a thread handed to Bob: what Bob answers them reaches
/// Ada as a note in Bob's words, heard at her next turn, and the person's
/// own line never does.
#[tokio::test]
async fn a_persons_turn_in_a_handed_over_thread_leaves_its_opener_the_colleagues_words() {
    let log = scratch("aside-persons-turn");
    enrol(&log, &persona("ada"));
    let mut bob = persona("bob");
    bob.name = "Bob".into();
    enrol(&log, &bob);
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
    let bobs = Fake::new(
        Scripted::turns(vec![reply("The fix is in."), reply("Darker it is.")]).gated(gate.clone()),
    );
    let others = Fake::new(Scripted::turns(vec![reply("Thanks."), reply("Noted.")]));
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Apart {
            bob: bobs.clone(),
            others: others.clone(),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    until("bob is at work on it", || bobs.prompts().len() == 1).await;
    let thread = work_thread(&room, &id);
    room.prompt_side(&thread, "Make it darker", None)
        .await
        .unwrap();
    // The result first, heard by Ada before Bob answers the person.
    gate.add_permits(2);
    done(&room, &id).await;
    until("ada has heard her result", || {
        others
            .prompts()
            .iter()
            .any(|p| p.contains("The fix is in."))
    })
    .await;
    until("ada is at rest", || !room.mid_turn("ada")).await;
    gate.add_permits(2);
    until("bob has answered the person", || {
        asides(&room, "ada")
            .iter()
            .any(|aside| aside.level == AsideLevel::Note)
    })
    .await;

    let notes: Vec<Aside> = asides(&room, "ada")
        .into_iter()
        .filter(|aside| aside.level == AsideLevel::Note)
        .collect();
    assert_eq!(
        notes.len(),
        1,
        "one note, for the one turn that was not the result"
    );
    assert_eq!(notes[0].text, "Darker it is.");
    assert!(
        asides(&room, "ada")
            .iter()
            .all(|aside| !aside.text.contains("Make it darker")),
        "an aside carries the colleague's words, never the person's"
    );
    assert!(
        room.tape("ada")
            .iter()
            .all(|v| !v.to_string().contains("Darker it is.")),
        "nothing of it is in Ada's chat"
    );

    room.prompt("ada", "Anything new?", None, None)
        .await
        .unwrap();
    until("ada's next turn", || {
        others.prompts().iter().any(|p| p.contains("Anything new?"))
    })
    .await;
    let heard = others
        .told()
        .into_iter()
        .find(|p| p.contains("Anything new?"))
        .unwrap();
    assert!(
        heard.contains(&format!("Bob, in side:{thread}: Darker it is.")),
        "{heard}"
    );
    assert!(!heard.contains("Make it darker"), "{heard}");
}

/// The collaboration card on `whose` tape the person has not answered.
fn open_card(room: &Room, whose: &str) -> Option<serde_json::Value> {
    room.tape(whose).into_iter().find(|v| {
        v["kind"] == "permission"
            && v["requestId"]
                .as_str()
                .is_some_and(|id| id.starts_with(crate::session::peers::COLLAB_REQUEST_PREFIX))
            && v.get("decision").is_none()
    })
}

/// A room where Ada hands Bob work and Bob and Carol work in the ordinary
/// workspace reach, so Bob asking Carol needs the person's card; Bob's
/// handoff turn asks Carol and ends, held at the returned gate.
async fn asking_carol(name: &str, granted: bool, paused: bool) -> (Arc<Room>, String) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    for (id, name) in [("bob", "Bob"), ("carol", "Carol")] {
        enrol(
            &log,
            &Persona {
                name: name.into(),
                reach: Some(crate::contract::Reach::Workspace),
                ..persona(id)
            },
        );
    }
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(super::questions::turn(
            "asked",
            "I asked Carol which colour.",
            "end_turn",
        ))
        .gated(gate.clone()),
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Apart {
            bob: bob.clone(),
            others: Fake::new(Scripted::new(super::questions::turn(
                "heard", "Heard.", "end_turn",
            ))),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    if granted {
        room.allow_sender("carol", "bob").unwrap();
    }
    if paused {
        let pair = Pair {
            id: thread_key("bob", "carol").unwrap(),
            a: "bob".into(),
            b: "carol".into(),
            exchanges: EXCHANGE_CAP,
            paused: true,
            requests: vec![],
        };
        room.save_pair(&pair).unwrap();
        room.exchange_card(&pair, ExchangePauseStatus::Pending);
    }
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    TeammateTools::new(&room, "bob")
        .for_work(thread.clone())
        .call(
            "message_teammate",
            &json!({"to":"carol","message":"Which colour?","intent":"ask"}),
        )
        .await
        .unwrap();
    gate.add_permits(2);
    until("ada is left a note", || !asides(&room, "ada").is_empty()).await;
    (room, thread)
}

/// Bob may not ask Carol yet: his question waits on the person's card in his
/// main conversation, and the note the handoff's turn leaves Ada names it.
#[tokio::test]
async fn a_turn_waiting_on_a_question_the_person_must_allow_names_the_card() {
    let (room, _) = asking_carol("aside-collaboration-card", false, false).await;
    let notes: Vec<String> = asides(&room, "ada").into_iter().map(|a| a.text).collect();
    assert_eq!(
        notes,
        [
            "I asked Carol which colour.\n\nWaiting on the person: Allow Bob to ask Carol to work? The card is in Bob's main conversation."
        ]
    );
    until("the card is up in Bob's main conversation", || {
        open_card(&room, "bob").is_some()
    })
    .await;
}

/// Bob and Carol have paused at twelve: Bob's question waits on Keep going,
/// whatever his grant, and the note names the card that is up, never one for
/// allowing him to ask, which only comes once they go on.
#[tokio::test]
async fn a_turn_waiting_on_a_question_its_paused_pair_holds_names_the_pause() {
    for granted in [false, true] {
        let (room, _) =
            asking_carol(&format!("aside-paused-question-{granted}"), granted, true).await;
        let notes: Vec<String> = asides(&room, "ada").into_iter().map(|a| a.text).collect();
        assert_eq!(
            notes,
            [
                "I asked Carol which colour.\n\nWaiting on the person: Let Bob and Carol keep going? They paused at 12 messages. The card is in Bob's main conversation."
            ],
            "{granted}"
        );
        assert!(open_card(&room, "bob").is_none(), "{granted}");
    }
}

/// Bob may ask Carol, who is answering: the note says the turn waits on a
/// colleague's answer.
#[tokio::test]
async fn a_turn_waiting_on_a_colleagues_answer_says_so() {
    let (room, _) = asking_carol("aside-colleagues-answer", true, false).await;
    let notes: Vec<String> = asides(&room, "ada").into_iter().map(|a| a.text).collect();
    assert_eq!(
        notes,
        ["I asked Carol which colour.\n\nWaiting on a colleague's answer."]
    );
}

/// A handoff's turn that waits only on its question back to Ada leaves her
/// no note: the question reaches her by itself, and a note would say it
/// twice.
#[tokio::test]
async fn a_turn_waiting_only_on_a_question_to_its_sender_leaves_no_note() {
    let (room, _) = setup("aside-waits-on-the-sender");
    let mut handoff = saved_request("h", Intent::Handoff, Phase::WaitingHuman);
    handoff.thread = Some("t".into());
    handoff.started = true;
    handoff.human_actions = vec![HumanGate {
        id: "q".into(),
        consumed: false,
    }];
    let mut back = saved_request("q", Intent::Ask, Phase::Running);
    back.from = "bob".into();
    back.to = "ada".into();
    back.about = Some("h".into());
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: 0,
        paused: false,
        requests: vec![handoff, back],
    })
    .unwrap();
    let said = crate::session::runner::Driven {
        replies: vec!["I asked Ada which port.".into()],
        ..Default::default()
    };
    room.note_turn("t", true, &said);
    assert_eq!(asides(&room, "ada"), []);
}
