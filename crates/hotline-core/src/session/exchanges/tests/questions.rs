//! A colleague's question goes where the memory is: the recipient's main
//! conversation answers it when idle, a work thread of the recipient's
//! answers at once while it is busy and leaves it an aside, and a question
//! naming a thread is answered there. No session of the pair's own answers.
use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};
use crate::session::Agents;
use crate::thread::ThreadState;
use tokio::sync::Semaphore;

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

pub(super) fn request(room: &Room, key: &str, id: &str) -> Request {
    room.exchange_pair(key)
        .and_then(|pair| pair.requests.into_iter().find(|r| r.id == id))
        .expect("the request is saved")
}

/// What came back to Ada of the request `id`.
pub(super) fn answer_to(room: &Room, id: &str) -> Option<serde_json::Value> {
    room.tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
}

fn question_in(events: &[serde_json::Value], id: &str) -> Option<usize> {
    events
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id)
}

fn in_thread(room: &Room, thread: &str) -> Vec<serde_json::Value> {
    room.log.load(&StreamId::Side(thread.to_string()))
}

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

/// Bob's main conversation, Bob's work threads, and everyone else, each
/// from a script of its own.
struct ByThread {
    main: Arc<Fake>,
    work: Arc<Fake>,
    others: Arc<Fake>,
}
#[async_trait::async_trait]
impl Agents for ByThread {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        let agents = match (persona.id.as_str(), tools.in_work()) {
            ("bob", true) => &self.work,
            ("bob", false) => &self.main,
            _ => &self.others,
        };
        agents.agent(persona, preamble, said, tools, mcp)
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

#[tokio::test]
async fn an_ask_to_an_idle_main_conversation_is_answered_there() {
    let (room, agents) = setup("ask-idle-main");
    let id = send(&room, "ask").await;
    done(&room, &id).await;

    let bob = room.tape("bob");
    let question = question_in(&bob, &id).expect("the question is in Bob's main conversation");
    assert_eq!(bob[question]["cause"]["threadKey"], "ada~bob");
    assert_eq!(bob[question]["text"], "work ask");
    let reply = bob[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("Bob answers it there");
    assert_eq!(reply["text"], "result-0");
    let heard = &agents.prompts()[0];
    assert!(heard.contains("Ada, your teammate, asks you"), "{heard}");
    assert!(
        heard.contains("Hotline sends that reply back to Ada by itself"),
        "{heard}"
    );
    assert!(request(&room, "ada~bob", &id).thread.is_none());

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
}

/// The separate ask session is gone: an ask builds no agent of the pair's
/// own, only the main conversations it reaches, and the pair's thread has no
/// agent to build.
#[tokio::test]
async fn no_peer_session_or_peer_agent_is_built_for_an_ask() {
    let (room, agents) = setup("ask-no-peer-agent");
    let id = send(&room, "ask").await;
    done(&room, &id).await;
    until("Ada hears the answer", || {
        answer_to(&room, &id).is_some_and(|v| v["receipt"] == "read")
    })
    .await;
    let built: Vec<String> = agents.views().into_iter().map(|view| view.id).collect();
    assert_eq!(built, ["bob", "ada"], "only the two main conversations");
    assert!(
        agents
            .tools()
            .iter()
            .all(|tools| !tools.in_work() && !tools.in_run()),
        "every agent built is a main conversation's"
    );
    assert!(
        lock(&agents.preambles)
            .iter()
            .all(|preamble| !preamble.contains("replying privately")),
        "nothing is told it answers privately in a session of its own"
    );
    let pair = crate::thread::Policy::of(ThreadKind::Pair);
    assert_eq!(pair.tools, crate::thread::Tools::None);
    assert!(!pair.has_computer());
}

/// Decision 1: an ask never waits behind a busy main conversation. A work
/// thread of Bob's answers it at once, seeded with what his main conversation
/// knows (its tail, his recent asides, his thread index on the turn), hangs
/// off the pair's thread, closes with its answer, and leaves Bob's main
/// conversation an aside saying what it answered on his behalf.
#[tokio::test]
async fn a_busy_main_conversation_is_answered_for_by_a_seeded_work_thread_that_asides_back() {
    let log = scratch("ask-busy-main");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let main = Fake::new(
        Scripted::turns(vec![
            turn("person", "Done for the person.", "end_turn"),
            turn("next", "On it.", "end_turn"),
        ])
        .gated(gate.clone()),
    );
    let work = Fake::new(Scripted::new(turn("port", "The port is 8080.", "end_turn")));
    let others = Fake::new(Scripted::new(turn("heard", "Noted.", "end_turn")));
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(ByThread {
            main: main.clone(),
            work: work.clone(),
            others,
        }),
    );
    room.keep_aside(&Aside {
        id: "earlier".into(),
        ts: now_ms(),
        to: "bob".into(),
        from: AsideSource::Thread(ThreadId::side("elsewhere")),
        by: Some("ada".into()),
        text: "The staging box is down.".into(),
        about: None,
        level: AsideLevel::Note,
        heard: None,
        seen: None,
        looped_in: false,
        source_name: None,
    })
    .unwrap();
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("Bob's main conversation is busy", || {
        main.prompts().len() == 1 && room.mid_turn("bob")
    })
    .await;

    let id = send(&room, "ask").await;
    done(&room, &id).await;

    let asked = request(&room, "ada~bob", &id);
    assert!(asked.own_thread);
    let thread = asked.thread.expect("a work thread answered it");
    assert_eq!(
        answer_to(&room, &id).expect("the answer reaches Ada")["text"],
        "The port is 8080."
    );
    assert!(room.mid_turn("bob"), "the person's turn goes on");
    assert_eq!(
        main.prompts().len(),
        1,
        "the main conversation took nothing"
    );
    assert!(
        question_in(&room.tape("bob"), &id).is_none(),
        "nothing waits behind the busy turn"
    );

    let stream = in_thread(&room, &thread);
    let question = question_in(&stream, &id).expect("the question opened the thread");
    assert_eq!(stream[question]["text"], "work ask");
    let preamble = lock(&work.preambles)[0].clone();
    assert!(
        preamble.contains("Ada asked you the question that opened this thread"),
        "{preamble}"
    );
    assert!(preamble.contains("The staging box is down."), "{preamble}");
    let told = work.told()[0].clone();
    assert!(
        told.contains("THREADS · "),
        "the index opens its turn: {told}"
    );

    until("the thread closes with its answer", || {
        room.link_of(&ThreadId::side(&thread))
            .is_some_and(|link| matches!(link.state, ThreadState::Closed(_)))
    })
    .await;
    let link = room.link_of(&ThreadId::side(&thread)).unwrap();
    assert_eq!(link.asked_in.as_deref(), Some("ada~bob"));
    assert!(
        room.log
            .load(&StreamId::Pair("ada~bob".into()))
            .iter()
            .any(|v| v["kind"] == "link" && v["thread"] == thread.as_str()),
        "the thread hangs off the pair's"
    );

    let aside = crate::store::asides::of(&room.log, "bob")
        .into_iter()
        .find(|aside| aside.id == format!("answered:{id}"))
        .expect("the main conversation is left an aside");
    assert_eq!(aside.text, "Answered Ada for you: The port is 8080.");
    assert_eq!(aside.level, AsideLevel::Note);
    assert_eq!(aside.about, None, "heard in the main conversation");
    assert_eq!(aside.from, AsideSource::Thread(ThreadId::side(&thread)));
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["kind"] != "agent" || v["text"] != "The port is 8080."),
        "nothing of it is in the person's chat"
    );

    gate.add_permits(10);
    until("the person's turn ends", || !room.mid_turn("bob")).await;
    room.prompt("bob", "what's new?", None, None).await.unwrap();
    until("the next turn hears the aside", || {
        main.told()
            .last()
            .is_some_and(|told| told.contains("Answered Ada for you: The port is 8080."))
    })
    .await;
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
/// question too: it fails to its asker at once, once, and is never handed
/// over again.
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
        agents.prompts().len() == 1 && room.mid_turn("bob")
    })
    .await;
    // A question that waits behind the turn, as one does when Bob has no
    // place for a thread of its own.
    let mut asked = saved_request("queued-question", Intent::Ask, Phase::Running);
    asked.request_counted = true;
    seed(&room, asked, 1, false);
    room.deliver_to_main(
        "bob",
        "question:queued-question",
        DeliveryCause::Question {
            request_id: "queued-question".into(),
            persona_id: "ada".into(),
            name: "Ada".into(),
            thread_key: "ada~bob".into(),
            about: "saved work".into(),
            work: None,
        },
        "saved work".into(),
        Some(RequestLine {
            request: "queued-question".into(),
            answer: None,
        }),
        false,
    )
    .await
    .unwrap();

    room.cancel("bob").unwrap();
    room.wake_exchange("ada~bob");
    done(&room, "queued-question").await;
    let asked = request(&room, "ada~bob", "queued-question");
    assert!(asked.failed && !asked.started);
    let name = room.persona("bob").unwrap().name;
    let failure = answer_to(&room, "queued-question").expect("Ada is told at once");
    assert_eq!(failure["cause"]["status"], "failed");
    assert_eq!(
        failure["text"],
        format!(
            "{name} was stopped before answering this question. Send it again if you still need it."
        )
    );
    gate.add_permits(20);
    until("Ada has heard it", || {
        answer_to(&room, "queued-question").is_some_and(|v| v["receipt"] == "read")
    })
    .await;
    assert!(
        agents.prompts().iter().all(|p| !p.contains("asks you")),
        "no turn took the question up"
    );
    room.wake_exchange("ada~bob");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(agents.prompts().iter().all(|p| !p.contains("asks you")));
}

/// Ada's handoff to Bob, its result heard by her and its thread closed by it.
async fn handed_back(room: &Arc<Room>, agents: &Fake) -> (String, String) {
    let sent = hand_off(room, "ada", "bob", "Fix the build.", None)
        .await
        .unwrap();
    let handoff = sent["requestId"].as_str().unwrap().to_string();
    done(room, &handoff).await;
    let thread = work_thread(room, &handoff);
    until("Ada has heard the result and the thread has closed", || {
        agents.prompts().len() == 2 && !room.mid_turn("ada") && !room.work_is_open(&thread)
    })
    .await;
    (handoff, thread)
}

#[tokio::test]
async fn an_ask_naming_a_thread_is_answered_in_it_and_leaves_it_open() {
    let (room, agents) = setup("ask-in-thread");
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;

    let sent = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Which branch is it on?","intent":"ask","thread":handoff}),
        )
        .await
        .unwrap();
    let id = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    done(&room, &id).await;

    let asked = request(&room, "ada~bob", &id);
    assert_eq!(asked.thread.as_deref(), Some(thread.as_str()));
    assert!(!asked.own_thread);
    let stream = in_thread(&room, &thread);
    let question = question_in(&stream, &id).expect("the question is in the work thread");
    assert_eq!(stream[question]["cause"]["work"], "Fix the build.");
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
        heard.contains("Ada, your teammate, asks you about the work \"Fix the build.\""),
        "{heard}"
    );
    let answer = answer_to(&room, &id).expect("the answer reaches Ada");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], reply["text"]);
    assert!(
        room.work_is_open(&thread),
        "answering an ask does not close the thread"
    );
    assert!(
        question_in(&room.tape("bob"), &id).is_none(),
        "Bob's main conversation never heard it"
    );
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
    let adas = hand_off(&room, "ada", "bob", "Fix the build.", None)
        .await
        .unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    let carols = hand_off(&room, "carol", "bob", "Check the lint.", None)
        .await
        .unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    for (to, thread) in [
        ("bob", carols.as_str()),
        ("carol", adas.as_str()),
        ("bob", "nope"),
    ] {
        let refused = TeammateTools::new(&room, "ada")
            .call(
                "message_teammate",
                &json!({"to": to, "message": "How is it going?", "intent": "ask", "thread": thread}),
            )
            .await;
        assert!(
            refused
                .as_ref()
                .is_err_and(|error| error.contains("is not a handoff you sent")),
            "{to} in {thread}: {refused:?}"
        );
    }
    assert_eq!(room.exchange_pair("ada~bob").unwrap().requests.len(), 1);
}

/// Puts a teammate in the ordinary workspace reach, where asking a colleague
/// needs the person's grant.
fn in_workspace(room: &Room, id: &str) {
    let mut persona = room.persona(id).unwrap();
    persona.reach = Some(crate::contract::Reach::Workspace);
    crate::room::append_persona(&room.log, &persona).unwrap();
}

/// Bob asks Ada, the teammate that handed him the thread `thread`, from
/// inside it.
async fn ask_back(room: &Arc<Room>, thread: &str) -> String {
    let sent = TeammateTools::new(room, "bob")
        .for_work(thread.to_string())
        .call(
            "message_teammate",
            &json!({"to":"ada","message":"Which port?","intent":"ask"}),
        )
        .await
        .unwrap();
    serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A question from a handed-over thread to the teammate that handed it over
/// reaches the conversation that did, Ada's main one, with no card while the
/// delegation stands, and its answer returns to the thread. Once the
/// delegation is stopped, the same question needs the ordinary grant.
#[tokio::test]
async fn a_delegated_threads_question_to_its_opener_reaches_the_dm_without_a_card() {
    let (room, agents) = setup("ask-back-dm");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    room.start("ada").await.unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;
    room.continue_side(&thread).await.unwrap();

    let id = ask_back(&room, &thread).await;
    let asked = request(&room, "ada~bob", &id);
    assert_eq!(asked.about.as_deref(), Some(handoff.as_str()));
    done(&room, &id).await;
    let ada = room.tape("ada");
    let question = question_in(&ada, &id).expect("the question is in Ada's main conversation");
    assert_eq!(ada[question]["cause"]["work"], "Fix the build.");
    let reply = ada[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("Ada answers it there");
    let answer = in_thread(&room, &thread)
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id.as_str())
        .expect("the answer reaches the thread");
    assert_eq!(answer["text"], reply["text"]);
    let carded = |room: &Room| {
        ["ada", "bob"]
            .iter()
            .any(|whose| room.tape(whose).iter().any(|v| v["kind"] == "permission"))
    };
    assert!(!carded(&room), "no collaboration card was raised");

    room.stop_exchange("ada", "bob").unwrap();
    let again = ask_back(&room, &thread).await;
    until("the question waits on the person's card", || carded(&room)).await;
    assert!(request(&room, "ada~bob", &again).about.is_some());
    assert!(question_in(&room.tape("ada"), &again).is_none());
}

/// Bob's main conversation mid-turn on a tape whose previous chapter can be
/// reopened, with Ada's question waiting behind that turn, as one does when
/// Bob has no place for a thread of its own.
async fn asked_behind_bobs_turn(name: &str) -> (Arc<Room>, Arc<Semaphore>) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let t = now_ms() - 60_000;
    for event in [
        json!({"kind":"chapter","id":"c-old","ts":t,"backendId":"hotline","endedAt":t+3000,"title":"Old work","closedBy":"user"}),
        json!({"kind":"user","id":"u-old","ts":t+1000,"text":"old line"}),
        json!({"kind":"agent","id":"a-old","ts":t+2000,"text":"Old answer."}),
    ] {
        log.append(&StreamId::Tape("bob".into()), &event).unwrap();
    }
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(
        Scripted::turns(vec![
            turn("person", "Done for the person.", "end_turn"),
            turn("blue", "Blue.", "end_turn"),
        ])
        .gated(gate.clone()),
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("the person's turn is running", || {
        agents.prompts().len() == 1 && room.mid_turn("bob")
    })
    .await;
    let mut asked = saved_request("behind-the-turn", Intent::Ask, Phase::Running);
    asked.request_counted = true;
    seed(&room, asked, 1, false);
    room.deliver_to_main(
        "bob",
        "question:behind-the-turn",
        DeliveryCause::Question {
            request_id: "behind-the-turn".into(),
            persona_id: "ada".into(),
            name: "Ada".into(),
            thread_key: "ada~bob".into(),
            about: "saved work".into(),
            work: None,
        },
        "saved work".into(),
        Some(RequestLine {
            request: "behind-the-turn".into(),
            answer: None,
        }),
        false,
    )
    .await
    .unwrap();
    room.wake_exchange("ada~bob");
    (room, gate)
}

/// A chapter's swap mid-turn, Bob's agent reopening his previous chapter as
/// `resume_chapter` may or the person's next line starting one after
/// `new_chapter`, drops what waited behind the cut turn: the question among
/// it is handed to the session that replaced it, and the new chapter
/// answers it.
#[tokio::test]
async fn a_question_waiting_behind_a_turn_outlives_a_chapter_swap() {
    for reopened in [true, false] {
        let (room, gate) = asked_behind_bobs_turn(&format!("ask-behind-a-swap-{reopened}")).await;
        if reopened {
            room.resume_chapter("bob").await.unwrap();
        } else {
            room.start_fresh_chapter("bob", crate::contract::ChapterClose::Agent)
                .await
                .unwrap();
            room.prompt("bob", "the next chapter", None, None)
                .await
                .unwrap();
        }
        gate.add_permits(50);
        done(&room, "behind-the-turn").await;
        let answer = request(&room, "ada~bob", "behind-the-turn");
        assert!(!answer.failed, "{}", answer.reply);
        assert_eq!(answer.reply, "Blue.");
    }
}
