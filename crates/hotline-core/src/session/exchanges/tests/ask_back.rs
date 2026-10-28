//! A question a thread a colleague handed over asks the teammate that
//! handed it over is answered by the conversation that did, by that
//! handoff's reply route, without a card while the delegation stands; its
//! answer or its failure is heard in the thread, and a stop of it is only
//! shown there.
use super::gates::{Apart, ask_from, handed_off, idle, request, turn};
use super::progress::{Builds, planted_job};
use super::questions::{answer_to, copied, handed_back, in_thread, questions_heard, requested};
use super::*;
use crate::session::turns::{Line, RequestLine};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Semaphore;

/// Bob asks Ada, the teammate that handed him the thread `thread`, from
/// inside it.
pub(super) async fn ask_back(room: &Arc<Room>, thread: &str) -> String {
    let sent = TeammateTools::new(room, "bob")
        .for_work(thread.to_string())
        .call(
            "message_teammate",
            &json!({"to":"ada","message":"Which port?","intent":"ask"}),
        )
        .await
        .unwrap();
    requested(&serde_json::from_str(&sent).unwrap())
}

/// Puts a teammate in the ordinary workspace reach, where asking a colleague
/// needs the person's grant.
pub(super) fn in_workspace(room: &Room, id: &str) {
    let mut persona = room.persona(id).unwrap();
    persona.reach = Some(crate::contract::Reach::Workspace);
    crate::room::append_persona(&room.log, &persona).unwrap();
}

/// Whether a permission card was raised on Ada's or Bob's tape.
fn carded(room: &Room) -> bool {
    ["ada", "bob"]
        .iter()
        .any(|whose| room.tape(whose).iter().any(|v| v["kind"] == "permission"))
}

pub(super) fn question_in(events: &[serde_json::Value], id: &str) -> Option<usize> {
    events
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id)
}

fn answer_in(room: &Room, thread: &str, id: &str) -> Option<serde_json::Value> {
    in_thread(room, thread)
        .into_iter()
        .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
}

/// The question was answered by a turn of the conversation `events` is, in
/// the work it is about, and that answer is what reached the thread.
fn answered_there(room: &Room, events: &[serde_json::Value], thread: &str, id: &str) {
    let question = question_in(events, id).expect("the question is in that conversation");
    assert_eq!(events[question]["cause"]["work"], "Fix the build.");
    assert_eq!(events[question]["text"], "Which port?");
    let reply = events[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("a turn there answers it");
    let answer = answer_in(room, thread, id).expect("the answer reaches the thread");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], reply["text"]);
    assert!(!carded(room), "no collaboration card was raised");
}

/// Ada's handoff to Bob, sent with `tools` from a conversation of Ada's that
/// hears its result, and Bob's thread continued by the person once `heard`
/// says that conversation has heard it.
async fn handed_from(
    room: &Arc<Room>,
    tools: TeammateTools,
    heard: impl Fn() -> bool,
) -> (String, String) {
    let sent = tools
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Fix the build.","intent":"handoff"}),
        )
        .await
        .unwrap();
    let handoff = requested(&serde_json::from_str(&sent).unwrap());
    done(room, &handoff).await;
    let thread = work_thread(room, &handoff);
    until(
        "ada has heard the result and bob's thread has closed",
        || heard() && !room.work_is_open(&thread),
    )
    .await;
    room.continue_side(&thread).await.unwrap();
    (handoff, thread)
}

#[tokio::test]
async fn a_delegated_threads_question_to_its_opener_reaches_the_dm_without_a_card() {
    let (room, agents) = setup("ask-back-dm");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    room.start("ada").await.unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;
    room.continue_side(&thread).await.unwrap();

    let id = ask_back(&room, &thread).await;
    assert_eq!(
        request(&room, "ada~bob", &id).about.as_deref(),
        Some(handoff.as_str())
    );
    done(&room, &id).await;
    answered_there(&room, &room.tape("ada"), &thread, &id);
}

#[tokio::test]
async fn a_delegated_threads_question_to_its_opener_reaches_the_work_thread_it_was_handed_from() {
    let (room, agents) = setup("ask-back-work");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    let work = room
        .start_side("ada", "plan the release")
        .await
        .unwrap()
        .side_id;
    until("ada's work thread has taken its task", || {
        agents.prompts().len() == 1
    })
    .await;
    let tools = TeammateTools::new(&room, "ada").for_work(work.clone());
    let (_, thread) = handed_from(&room, tools, || {
        agents.prompts().len() == 3 && idle(&room, &work)
    })
    .await;

    let id = ask_back(&room, &thread).await;
    done(&room, &id).await;
    answered_there(&room, &in_thread(&room, &work), &thread, &id);
    assert!(
        question_in(&room.tape("ada"), &id).is_none(),
        "Ada's DM never heard it"
    );
}

#[tokio::test]
async fn a_delegated_threads_question_to_its_opener_reaches_the_job_it_was_handed_from() {
    let (room, agents) = setup("ask-back-job");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    let job = planted_job(&room, "job-build");
    room.prompt_job(&job.id, "watch it", None).await.unwrap();
    until("ada's job has had its turn", || {
        agents.prompts().len() == 1 && room.job_idle(&job.id)
    })
    .await;
    let tools = TeammateTools::new(&room, "ada")
        .with_capability(room.capability_lease("ada"))
        .for_job(job.id.clone());
    let (_, thread) = handed_from(&room, tools, || {
        agents.prompts().len() == 3 && room.job_idle(&job.id)
    })
    .await;

    let id = ask_back(&room, &thread).await;
    done(&room, &id).await;
    let stream = room.log.load(&StreamId::Job(job.id.clone()));
    answered_there(&room, &stream, &thread, &id);
    assert!(
        question_in(&room.tape("ada"), &id).is_none(),
        "Ada's DM never heard it"
    );
}

/// The card the person has not answered yet on `whose` tape.
pub(super) fn open_card(room: &Room, whose: &str) -> Option<serde_json::Value> {
    room.tape(whose)
        .into_iter()
        .find(|v| v["kind"] == "permission" && v.get("decision").is_none())
}

#[tokio::test]
async fn a_question_back_after_the_delegation_was_stopped_needs_the_ordinary_grant() {
    let (room, agents) = setup("ask-back-released");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;
    room.stop_exchange("ada", "bob").unwrap();
    assert!(request(&room, "ada~bob", &handoff).released);
    room.continue_side(&thread).await.unwrap();

    let id = ask_back(&room, &thread).await;
    until("the question waits on the card for bob to ask ada", || {
        open_card(&room, "bob").is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(question_in(&room.tape("ada"), &id).is_none());
    let card = open_card(&room, "bob").unwrap();
    room.answer_permission("bob", card["requestId"].as_str().unwrap(), "allow_always")
        .await
        .unwrap();

    done(&room, &id).await;
    let ada = room.tape("ada");
    let question = question_in(&ada, &id).expect("it reaches Ada's main conversation");
    assert!(
        ada[question]["cause"]["work"].is_null(),
        "{}",
        ada[question]
    );
    let answer = answer_in(&room, &thread, &id).expect("the answer reaches the thread");
    assert_eq!(answer["cause"]["status"], "done");
}

/// Ada's agents beside Bob's, apart, so the stop a closing thread of Bob's
/// gives his agent never reaches Ada's; none of Ada's starts once `down` is
/// set.
struct AdaApart {
    agents: Apart,
    down: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl Agents for AdaApart {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        if persona.id == "ada" && self.down.load(Ordering::SeqCst) {
            return Err("the harness is down".into());
        }
        self.agents.agent(persona, preamble, said, tools, mcp)
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

/// A room of Ada and Bob on agents of their own, Ada's turns waiting at the
/// gate.
struct Separate {
    room: Arc<Room>,
    ada: Arc<Fake>,
    bob: Arc<Fake>,
    gate: Arc<Semaphore>,
    ada_down: Arc<AtomicBool>,
}

fn apart(name: &str, ada_turns: Vec<Vec<Update>>) -> Separate {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let ada = Fake::new(Scripted::turns(ada_turns).gated(gate.clone()));
    let bob = Fake::new(Scripted::turns(
        (0..8)
            .map(|i| turn(&format!("bob-{i}"), &format!("Bob {i}."), "end_turn"))
            .collect(),
    ));
    let ada_down = Arc::new(AtomicBool::new(false));
    let agents = Arc::new(AdaApart {
        agents: Apart {
            bob: bob.clone(),
            others: ada.clone(),
        },
        down: ada_down.clone(),
    });
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    Separate {
        room,
        ada,
        bob,
        gate,
        ada_down,
    }
}

/// The line a thread shows when a question it asked was stopped.
fn stopped_line(room: &Room, thread: &str, id: &str) -> Option<serde_json::Value> {
    in_thread(room, thread)
        .into_iter()
        .find(|v| v["kind"] == "notice" && v["id"] == format!("exchange-stopped:{id}"))
}

/// Ada's turn answering the question goes on to its end, and the thread that
/// asked shows that the question was stopped in a line that starts no turn
/// there: the stop does not set Bob going again, to send it anew.
#[tokio::test]
async fn stop_exchange_leaves_the_delegators_turn_alone_and_fails_the_question() {
    let Separate {
        room,
        ada,
        bob,
        gate,
        ..
    } = apart(
        "ask-back-stopped",
        vec![
            turn("noted", "Noted.", "end_turn"),
            turn("port", "The port is 8080.", "end_turn"),
        ],
    );
    gate.add_permits(2);
    let (_, thread) = handed_from(&room, TeammateTools::new(&room, "ada"), || {
        ada.prompts().len() == 1 && !room.mid_turn("ada")
    })
    .await;
    let id = ask_back(&room, &thread).await;
    until("Ada's turn answering the question is running", || {
        request(&room, "ada~bob", &id).started && room.mid_turn("ada")
    })
    .await;
    let prompted = bob.prompts().len();
    let sent = room.exchange_pair("ada~bob").unwrap().requests.len();

    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(ada.cancel_count(), 0, "Ada's turn is not cancelled");
    assert!(room.mid_turn("ada"));
    let stopped = request(&room, "ada~bob", &id);
    assert_eq!(stopped.phase, Phase::Stopped);
    assert!(stopped.failed);

    until("the thread shows the question was stopped", || {
        stopped_line(&room, &thread, &id).is_some()
    })
    .await;
    let line = stopped_line(&room, &thread, &id).unwrap();
    assert_eq!(line["level"], "info");
    assert!(
        line["text"]
            .as_str()
            .unwrap()
            .contains("The person stopped this exchange."),
        "{line}"
    );
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["id"] != format!("exchange-ended:{id}")),
        "Bob's DM is not told"
    );

    gate.add_permits(10);
    until("Ada's turn ends", || !room.mid_turn("ada")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        room.tape("ada")
            .iter()
            .any(|v| v["kind"] == "agent" && v["text"] == "The port is 8080."),
        "her turn ended as it would have"
    );
    assert!(
        answer_in(&room, &thread, &id).is_none(),
        "nothing about the question is delivered to the thread's agent"
    );
    assert_eq!(bob.prompts().len(), prompted, "no turn ran in the thread");
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests.len(),
        sent,
        "nothing was sent again"
    );
    let settled = request(&room, "ada~bob", &id);
    assert_eq!(settled.phase, Phase::Stopped);
    assert!(!settled.owed_to_thread);
}

/// Revoking the delegator stops the question, and the thread that asked it
/// shows so in a line that starts no turn. Stopping the asker himself tells
/// him in his DM instead, and wakes nothing of his.
#[tokio::test]
async fn a_revoked_question_back_is_shown_in_its_thread_unless_its_asker_was_stopped() {
    for asker_stopped in [false, true] {
        let Separate {
            room,
            ada,
            bob,
            gate,
            ..
        } = apart(
            &format!("ask-back-revoked-{asker_stopped}"),
            vec![
                turn("noted", "Noted.", "end_turn"),
                turn("port", "The port is 8080.", "end_turn"),
            ],
        );
        gate.add_permits(2);
        let (_, thread) = handed_from(&room, TeammateTools::new(&room, "ada"), || {
            ada.prompts().len() == 1 && !room.mid_turn("ada")
        })
        .await;
        let id = ask_back(&room, &thread).await;
        until("Ada's turn answering the question is running", || {
            request(&room, "ada~bob", &id).started && room.mid_turn("ada")
        })
        .await;
        let prompted = bob.prompts().len();

        if asker_stopped {
            room.stop("bob").unwrap();
        } else {
            room.invalidate("ada").unwrap();
        }
        assert_eq!(request(&room, "ada~bob", &id).phase, Phase::Stopped);
        let told_in_dm = || {
            room.tape("bob")
                .iter()
                .any(|v| v["id"] == format!("exchange-ended:{id}"))
        };
        if asker_stopped {
            assert!(told_in_dm(), "Bob is told in his DM");
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(stopped_line(&room, &thread, &id).is_none());
            assert!(room.sides.get(&thread).is_none(), "his thread stays down");
        } else {
            until("the thread shows the question was stopped", || {
                stopped_line(&room, &thread, &id).is_some()
            })
            .await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(!told_in_dm());
            assert_eq!(bob.prompts().len(), prompted, "no turn ran in the thread");
        }
        assert!(answer_in(&room, &thread, &id).is_none());
        gate.add_permits(10);
    }
}

#[tokio::test]
async fn a_handoff_that_asks_its_opener_waits_for_that_answer_without_reporting_it() {
    let handed = handed_off("ask-back-from-the-handoff", "end_turn").await;
    let room = &handed.room;
    let ask = ask_from(room, &handed.thread, "ada").await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on Ada and its turn has ended", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && idle(room, &handed.thread)
    })
    .await;
    until("the question reaches Ada's DM", || {
        question_in(&room.tape("ada"), &ask).is_some()
    })
    .await;
    let ada = room.tape("ada");
    assert_eq!(
        ada[question_in(&ada, &ask).unwrap()]["cause"]["work"],
        "work handoff"
    );
    assert!(
        notices(room, "ada").is_empty(),
        "the question is what Ada hears"
    );
    assert!(!carded(room));

    handed.carol_gate.add_permits(2);
    handed.bob_gate.add_permits(2);
    done(room, &handed.handoff).await;
    let answer = answer_in(room, &handed.thread, &ask).expect("Ada's answer reaches the thread");
    assert_eq!(answer["text"], "Use the blue one.");
    let result = answer_to(room, &handed.handoff).expect("the result reaches Ada");
    assert_eq!(result["cause"]["status"], "done");
    assert_eq!(result["text"], "Painted it blue.");
    assert_eq!(notices(room, "ada"), [] as [String; 0]);
}

/// Ada's job, a handoff from it to Bob that the job has heard the result of,
/// and Bob's thread continued by the person.
struct JobHandedBack {
    separate: Separate,
    job: String,
    thread: String,
}

async fn job_handed_back(name: &str) -> JobHandedBack {
    let turns = (0..8)
        .map(|i| turn(&format!("ada-{i}"), &format!("Ada {i}."), "end_turn"))
        .collect();
    let separate = apart(name, turns);
    let Separate {
        room, ada, gate, ..
    } = &separate;
    let job = planted_job(room, "job-build");
    gate.add_permits(4);
    room.prompt_job(&job.id, "watch it", None).await.unwrap();
    until("ada's job has had its turn", || {
        ada.prompts().len() == 1 && room.job_idle(&job.id)
    })
    .await;
    let tools = TeammateTools::new(room, "ada")
        .with_capability(room.capability_lease("ada"))
        .for_job(job.id.clone());
    let (_, thread) = handed_from(room, tools, || {
        ada.prompts().len() == 2 && room.job_idle(&job.id)
    })
    .await;
    JobHandedBack {
        separate,
        job: job.id,
        thread,
    }
}

/// The person's turn in Ada's job, still running, with Bob's question
/// waiting behind it.
async fn asked_behind_the_person(handed: &JobHandedBack) -> String {
    let Separate { room, ada, .. } = &handed.separate;
    room.prompt_job(&handed.job, "and now?", None)
        .await
        .unwrap();
    until("the person's turn in the job is running", || {
        ada.prompts().len() == 3
    })
    .await;
    let id = ask_back(room, &handed.thread).await;
    until("the question waits behind it", || {
        question_in(&room.log.load(&StreamId::Job(handed.job.clone())), &id).is_some()
    })
    .await;
    id
}

/// The question failed to Bob's thread once, as the person's cancel fails
/// one, and no turn of Ada's took it up.
async fn failed_once_unheard(handed: &JobHandedBack, id: &str) {
    let Separate {
        room, ada, gate, ..
    } = &handed.separate;
    done(room, id).await;
    let asked = request(room, "ada~bob", id);
    assert!(asked.failed && !asked.started);
    let failure = answer_in(room, &handed.thread, id).expect("Bob's thread is told at once");
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
    assert_eq!(questions_heard(ada), 0, "no turn took the question up");
    let told = in_thread(room, &handed.thread)
        .into_iter()
        .filter(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
        .count();
    assert_eq!(told, 1);
}

/// The person's Stop in a job drops what waited behind its turn, a
/// colleague's question too: it fails to its asker at once.
#[tokio::test]
async fn the_persons_stop_in_a_job_fails_a_question_waiting_behind_its_turn() {
    let handed = job_handed_back("ask-back-job-stopped").await;
    let id = asked_behind_the_person(&handed).await;
    handed.separate.room.cancel_job_turn(&handed.job).unwrap();
    failed_once_unheard(&handed, &id).await;
}

/// A job that ends, cancelled by the person or done on its goal, with Bob's
/// question still waiting behind its turn fails it to him once.
#[tokio::test]
async fn a_question_waiting_in_a_job_that_ends_fails_once_to_its_asker() {
    for by_itself in [false, true] {
        let handed = job_handed_back(&format!("ask-back-job-ends-{by_itself}")).await;
        let id = asked_behind_the_person(&handed).await;
        let room = &handed.separate.room;
        if by_itself {
            room.schedule_done(&handed.job, "Watched it.").unwrap();
        } else {
            room.schedule_cancel(&handed.job).unwrap();
        }
        failed_once_unheard(&handed, &id).await;
    }
}

/// A question that reaches a job as the person cancels it, behind the turn
/// being stopped, fails to its asker when that turn ends rather than waiting
/// in a job nothing runs any more.
#[tokio::test]
async fn a_question_queued_behind_a_cancelled_jobs_turn_fails_to_its_asker() {
    let handed = job_handed_back("ask-back-job-cancelled-under-it").await;
    let Separate {
        room, ada, gate, ..
    } = &handed.separate;
    room.prompt_job(&handed.job, "and now?", None)
        .await
        .unwrap();
    until("the person's turn in the job is running", || {
        ada.prompts().len() == 3
    })
    .await;
    let mut asked = saved_request("asked-as-it-ends", Intent::Ask, Phase::Running);
    asked.from = "bob".into();
    asked.to = "ada".into();
    asked.reply_thread = Some(handed.thread.clone());
    let mut pair = room.exchange_pair("ada~bob").unwrap();
    pair.requests.push(asked);
    room.save_pair(&pair).unwrap();
    let live = lock(&room.job_threads).get(&handed.job).cloned().unwrap();

    room.schedule_cancel(&handed.job).unwrap();
    room.queue_job(
        &live,
        crate::session::job_thread::JobLine {
            line: Line {
                voice: None,
                text: "Which port?".into(),
                attachments: Vec::new(),
                request: Some(RequestLine {
                    request: "asked-as-it-ends".into(),
                    answer: None,
                }),
                from: None,
                delivery: None,
                wake: false,
            },
            firing: None,
            last: false,
        },
    )
    .unwrap();
    gate.add_permits(20);
    done(room, "asked-as-it-ends").await;
    let asked = request(room, "ada~bob", "asked-as-it-ends");
    assert!(asked.failed && !asked.started);
}

/// Stopping or revoking Ada while her job's turn runs with a line waiting
/// behind it lets go of the job's agent at once, and drops the line with the
/// turn: it never runs, on that agent or on a new one.
#[tokio::test]
async fn stopping_or_revoking_a_teammate_lets_go_of_its_job_agent_with_a_line_waiting() {
    for revoke in [false, true] {
        let handed = job_handed_back(&format!("ask-back-job-let-go-{revoke}")).await;
        let Separate {
            room, ada, gate, ..
        } = &handed.separate;
        room.prompt_job(&handed.job, "and now?", None)
            .await
            .unwrap();
        until("the person's turn in the job is running", || {
            ada.prompts().len() == 3
        })
        .await;
        room.prompt_job(&handed.job, "one more thing", None)
            .await
            .unwrap();

        if revoke {
            room.invalidate("ada").unwrap();
        } else {
            room.stop("ada").unwrap();
        }
        assert!(
            room.link_of(&ThreadId::job(&handed.job))
                .is_some_and(|link| matches!(link.state, crate::thread::ThreadState::Parked)),
            "the job's agent is let go at once"
        );
        gate.add_permits(20);
        until("the job's turn ends", || room.job_idle(&handed.job)).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            ada.prompts()
                .iter()
                .all(|prompt| !prompt.contains("one more thing")),
            "the waiting line never runs: {:?}",
            ada.prompts()
        );
    }
}

/// A job whose agent cannot start for Bob's question fails it to him at
/// once, rather than leaving it waiting for a restart.
#[tokio::test]
async fn a_question_a_job_cannot_start_an_agent_for_fails_to_its_asker() {
    let handed = job_handed_back("ask-back-job-down").await;
    let room = &handed.separate.room;
    room.park_job(&handed.job);
    handed.separate.ada_down.store(true, Ordering::SeqCst);
    let id = ask_back(room, &handed.thread).await;
    failed_once_unheard(&handed, &id).await;
}

/// A question that reached a job but no turn of it when the desk stopped is
/// handed over once by the pair, with the ask its turn answers.
#[tokio::test]
async fn a_question_a_job_had_not_heard_at_a_restart_is_answered_once() {
    let handed = job_handed_back("ask-back-job-restart").await;
    let id = asked_behind_the_person(&handed).await;
    let old = &handed.separate.room;
    let log = copied(old, "ask-back-job-restarted");
    for stream in [
        StreamId::Job(handed.job.clone()),
        StreamId::Side(handed.thread.clone()),
    ] {
        for event in old.log.load(&stream) {
            log.append(&stream, &event).unwrap();
        }
    }

    let agents = Fake::new(Scripted::new(turn("port", "The port is 8080.", "end_turn")));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_exchanges().await;
    done(&room, &id).await;
    let answer = answer_in(&room, &handed.thread, &id).expect("the answer reaches the thread");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], "The port is 8080.");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(questions_heard(&agents), 1, "heard once");
}

/// Stop exchange while the handoff that asked its opener is mid-turn closes
/// the handoff's thread, and the question's failure does not open it again:
/// no turn runs there, and Bob hears of it in his DM.
#[tokio::test]
async fn stop_exchange_while_the_asking_handoff_works_leaves_its_thread_closed() {
    let handed = handed_off("ask-back-stopped-mid-turn", "end_turn").await;
    let room = &handed.room;
    let ask = ask_from(room, &handed.thread, "ada").await;
    until("the question reaches Ada's DM", || {
        question_in(&room.tape("ada"), &ask).is_some()
    })
    .await;

    room.stop_exchange("ada", "bob").unwrap();
    until("Bob hears of the question in his DM", || {
        room.tape("bob")
            .iter()
            .any(|v| v["id"] == format!("exchange-ended:{ask}"))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !room.work_is_open(&handed.thread),
        "the thread stays closed"
    );
    assert_eq!(handed.bob.prompts().len(), 1, "no turn ran in it");
    assert!(answer_in(room, &handed.thread, &ask).is_none());
    assert!(!request(room, "ada~bob", &ask).owed_to_thread);
    handed.bob_gate.add_permits(10);
    handed.carol_gate.add_permits(10);
}

/// Ada's work thread, a handoff from it to Bob whose turn is held, and Bob's
/// question back to Ada from that turn, delivered into her work thread.
struct AskedIntoWork {
    room: Arc<Room>,
    ada_gate: Arc<Semaphore>,
    bob_gate: Arc<Semaphore>,
    work: String,
    handoff: String,
    thread: String,
    ask: String,
}

/// `behind_the_person`: the person's turn in Ada's work thread is running,
/// so the question waits behind it; else a turn there is answering it.
async fn asked_into_work(name: &str, behind_the_person: bool) -> AskedIntoWork {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let (ada_gate, bob_gate) = (Arc::new(Semaphore::new(0)), Arc::new(Semaphore::new(0)));
    let ada = Fake::new(
        Scripted::turns(
            (0..6)
                .map(|i| turn(&format!("ada-{i}"), &format!("Ada {i}."), "end_turn"))
                .collect(),
        )
        .gated(ada_gate.clone()),
    );
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("asked", "I asked Ada which port.", "end_turn"),
            turn("built", "Built it on 8080.", "end_turn"),
        ])
        .gated(bob_gate.clone()),
    );
    let agents = Arc::new(Apart {
        bob: bob.clone(),
        others: ada.clone(),
    });
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.allow_sender("bob", "ada").unwrap();
    let work = room
        .start_side("ada", "plan the release")
        .await
        .unwrap()
        .side_id;
    ada_gate.add_permits(2);
    until("ada's work thread has had its turn", || {
        ada.prompts().len() == 1 && idle(&room, &work)
    })
    .await;
    let sent = TeammateTools::new(&room, "ada")
        .for_work(work.clone())
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Fix the build.","intent":"handoff"}),
        )
        .await
        .unwrap();
    let handoff = requested(&serde_json::from_str(&sent).unwrap());
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    if behind_the_person {
        room.prompt_side(&work, "the person's work", None)
            .await
            .unwrap();
        until("the person's turn in the work thread is running", || {
            ada.prompts().len() == 2
        })
        .await;
    }
    let ask = ask_from(&room, &thread, "ada").await;
    until("the question is in the work thread", || {
        question_in(&in_thread(&room, &work), &ask).is_some()
    })
    .await;
    if !behind_the_person {
        until("a turn there is answering it", || {
            request(&room, "ada~bob", &ask).started
        })
        .await;
    }
    AskedIntoWork {
        room,
        ada_gate,
        bob_gate,
        work,
        handoff,
        thread,
        ask,
    }
}

/// A question waiting in Ada's work thread, or being answered there, when
/// that thread is archived is settled anyway: Bob's thread hears that it
/// failed, and his handoff, which waits on it, carries on to its result.
#[tokio::test]
async fn a_question_in_a_work_thread_that_is_archived_fails_and_the_handoff_carries_on() {
    for behind_the_person in [true, false] {
        let asked = asked_into_work(
            &format!("ask-back-archived-{behind_the_person}"),
            behind_the_person,
        )
        .await;
        let room = &asked.room;
        room.archive_side(&asked.work, crate::contract::SideEnd::Person, None)
            .unwrap();
        until("the question is settled", || {
            request(room, "ada~bob", &asked.ask).phase != Phase::Running
        })
        .await;
        asked.ada_gate.add_permits(20);
        asked.bob_gate.add_permits(20);
        done(room, &asked.handoff).await;

        let settled = request(room, "ada~bob", &asked.ask);
        assert_eq!(settled.phase, Phase::Done);
        let failure =
            answer_in(room, &asked.thread, &asked.ask).expect("Bob's thread hears what came of it");
        assert_eq!(failure["cause"]["status"], "failed");
        if behind_the_person {
            assert!(!settled.started);
            assert!(
                failure["text"]
                    .as_str()
                    .unwrap()
                    .contains("was stopped before answering this question"),
                "{failure}"
            );
        }
        let result = answer_to(room, &asked.handoff).expect("the result reaches Ada");
        assert_eq!(result["cause"]["status"], "done");
        assert_eq!(result["text"], "Built it on 8080.");
    }
}

/// A question the delegation was the grant for, whose delegation ends
/// before it is handed over, fails to its asker: it never reaches Ada's main
/// conversation without a card.
#[tokio::test]
async fn a_question_whose_delegation_lapses_before_it_is_handed_over_fails() {
    let (room, agents) = setup("ask-back-lapsed-before-hand-over");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;
    let mut asked = saved_request("asked-back", Intent::Ask, Phase::Running);
    asked.from = "bob".into();
    asked.to = "ada".into();
    asked.message = "Which port?".into();
    asked.request_counted = true;
    asked.reply_thread = Some(thread.clone());
    asked.about = Some(handoff.clone());
    let mut pair = room.exchange_pair("ada~bob").unwrap();
    pair.requests.push(asked.clone());
    room.save_pair(&pair).unwrap();
    lock(&room.exchange_leases)
        .get(&handoff)
        .expect("the handoff's leases")
        .0
        .revoke();

    room.hand_over_request("ada~bob", &asked, None, true)
        .await
        .unwrap();
    let failed = request(&room, "ada~bob", "asked-back");
    assert_eq!(failed.phase, Phase::Reply);
    assert!(failed.failed);
    assert!(
        question_in(&room.tape("ada"), "asked-back").is_none(),
        "Ada's main conversation never hears it"
    );
    room.wake_exchange("ada~bob");
    done(&room, "asked-back").await;
    let told = answer_in(&room, &thread, "asked-back").expect("the thread hears it failed");
    assert_eq!(told["cause"]["status"], "failed");
}

/// A stopped question no handoff waits on is shown in its parked thread even
/// when all of Bob's places are mid-turn: the line needs no turn, so the
/// thread stays parked, Bob's DM is not told, and nothing else between the
/// two is held up.
#[tokio::test]
async fn a_stopped_question_is_shown_in_its_parked_thread_with_no_place_free() {
    let log = scratch("ask-back-no-place");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let (ada_gate, bob_gate) = (Arc::new(Semaphore::new(0)), Arc::new(Semaphore::new(0)));
    let gated_turns = |who: &str| {
        (0..8)
            .map(|i| turn(&format!("{who}-{i}"), &format!("{who} {i}."), "end_turn"))
            .collect()
    };
    let ada = Fake::new(Scripted::turns(gated_turns("ada")).gated(ada_gate.clone()));
    // An agent of his own for each of Bob's threads, so the stop one closing
    // thread gives its agent reaches no other.
    let bob = (0..6)
        .map(|_| Fake::new(Scripted::turns(gated_turns("bob")).gated(bob_gate.clone())))
        .collect();
    let agents = Arc::new(Builds {
        ada: ada.clone(),
        bob,
        built: Mutex::new(0),
    });
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.allow_sender("bob", "ada").unwrap();
    ada_gate.add_permits(2);
    bob_gate.add_permits(2);
    let (_, thread) = handed_from(&room, TeammateTools::new(&room, "ada"), || {
        ada.prompts().len() == 1 && !room.mid_turn("ada")
    })
    .await;
    let ask = ask_back(&room, &thread).await;
    until("Ada's turn answering the question is running", || {
        request(&room, "ada~bob", &ask).started && room.mid_turn("ada")
    })
    .await;
    room.park_side_thread(&thread).unwrap();
    for chore in ["sweep", "dust", "mop"] {
        room.start_side("bob", chore).await.unwrap();
    }
    until("all of Bob's places are mid-turn", || {
        room.sides
            .all()
            .iter()
            .filter(|side| side.working())
            .count()
            == crate::session::sides::MAX_LIVE
    })
    .await;

    room.stop_exchange("ada", "bob").unwrap();
    until("the thread shows the question was stopped", || {
        stopped_line(&room, &thread, &ask).is_some()
    })
    .await;
    until("the pair's worker rests", || {
        !lock(&room.exchange_workers).contains("ada~bob")
    })
    .await;
    assert!(!request(&room, "ada~bob", &ask).owed_to_thread);
    assert!(answer_in(&room, &thread, &ask).is_none());
    assert!(room.sides.get(&thread).is_none(), "the thread stays parked");
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["id"] != format!("exchange-ended:{ask}")),
        "Bob's DM is not told"
    );
    ada_gate.add_permits(20);
    bob_gate.add_permits(20);
}

/// Ada, Bob and Carol, every agent on one script, none of them gated.
fn three(name: &str) -> (Arc<Room>, Arc<Fake>) {
    let log = scratch(name);
    for id in ["ada", "bob", "carol"] {
        enrol(&log, &persona(id));
    }
    let agents = Fake::new(Scripted::turns(
        (0..50)
            .map(|i| turn(&format!("said-{i}"), &format!("Said {i}."), "end_turn"))
            .collect(),
    ));
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}

/// Only a question a delegated thread sends the teammate that handed it
/// over is about that handoff. Asked from Bob's DM, from a thread of his own,
/// or from the delegated thread to someone else, it meets the ordinary
/// grant: a card, for a teammate in the workspace reach.
#[tokio::test]
async fn a_question_not_asked_back_meets_the_ordinary_grant() {
    let (room, agents) = three("ask-not-back");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    let (_, thread) = handed_back(&room, &agents).await;
    room.continue_side(&thread).await.unwrap();
    let own = room
        .start_side("bob", "tidy the docs")
        .await
        .unwrap()
        .side_id;
    for (tools, to, key) in [
        (TeammateTools::new(&room, "bob"), "ada", "ada~bob"),
        (
            TeammateTools::new(&room, "bob").for_work(own.clone()),
            "ada",
            "ada~bob",
        ),
        (
            TeammateTools::new(&room, "bob").for_work(thread.clone()),
            "carol",
            "bob~carol",
        ),
    ] {
        let sent = tools
            .call(
                "message_teammate",
                &json!({"to": to, "message": "Which port?", "intent": "ask"}),
            )
            .await
            .unwrap();
        let id = requested(&serde_json::from_str(&sent).unwrap());
        assert!(request(&room, key, &id).about.is_none(), "{to}");
        until("the question waits on the card", || {
            open_card(&room, "bob").is_some()
        })
        .await;
        assert!(question_in(&room.tape(to), &id).is_none(), "{to}");
        let card = open_card(&room, "bob").unwrap();
        room.answer_permission("bob", card["requestId"].as_str().unwrap(), "deny")
            .await
            .unwrap();
        until("the refused question is settled", || {
            request(&room, key, &id).phase == Phase::Done
        })
        .await;
        assert!(request(&room, key, &id).failed);
    }
}

/// A delegation whose leases ended, as a revocation's do before its record
/// says so, grants nothing: the question back meets the ordinary grant.
#[tokio::test]
async fn a_question_back_whose_delegation_lapsed_by_lease_meets_the_ordinary_grant() {
    let (room, agents) = setup("ask-back-lease-lapsed");
    in_workspace(&room, "bob");
    room.allow_sender("bob", "ada").unwrap();
    let (handoff, thread) = handed_back(&room, &agents).await;
    room.continue_side(&thread).await.unwrap();
    lock(&room.exchange_leases)
        .get(&handoff)
        .expect("the handoff's leases")
        .0
        .revoke();
    assert!(!request(&room, "ada~bob", &handoff).released);

    let id = ask_back(&room, &thread).await;
    assert_eq!(
        request(&room, "ada~bob", &id).about.as_deref(),
        Some(handoff.as_str())
    );
    until("the question waits on the card", || {
        open_card(&room, "bob").is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(question_in(&room.tape("ada"), &id).is_none());
}
