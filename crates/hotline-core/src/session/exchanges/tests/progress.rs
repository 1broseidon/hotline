//! Each turn of a thread a teammate handed over that is not its result
//! reaches the teammate that handed it over as a report: shown at once, kept
//! until heard, and heard once, one summary per thread. A report that may
//! wake its delegator wakes it once when it is idle.
use super::ask_back::open_card;
use super::gates::{ask_carol, ask_from, handed_off, idle, request, turn};
use super::*;
use crate::contract::{End, ScheduleKind, ScheduledJob, SideEnd};
use crate::session::exchanges::linked::take_sent;
use crate::session::exchanges::progress::{REPORT_CHARS, TurnEnd, WAKE_LINE, release, turn_report};
use crate::session::runner::Driven;
use crate::session::{Wired, timed};

/// Ada's agents, and Bob's one for each of his threads in the order they are
/// built, the last standing for every one after it: a script and a gate per
/// thread, so one thread's turn ends while another's still runs.
pub(super) struct Builds {
    pub(super) ada: Arc<Fake>,
    pub(super) bob: Vec<Arc<Fake>>,
    pub(super) built: Mutex<usize>,
}
#[async_trait::async_trait]
impl Agents for Builds {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        if persona.id != "bob" {
            return self.ada.agent(persona, preamble, said, tools, mcp);
        }
        let mut built = lock(&self.built);
        let bob = &self.bob[(*built).min(self.bob.len() - 1)];
        *built += 1;
        bob.agent(persona, preamble, said, tools, mcp)
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

pub(super) fn room_with(name: &str, ada: &Arc<Fake>, bob: Vec<Arc<Fake>>) -> Arc<Room> {
    let log = scratch(name);
    for id in ["ada", "bob"] {
        enrol(&log, &persona(id));
    }
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Builds {
            ada: ada.clone(),
            bob,
            built: Mutex::new(0),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    room
}

/// A handoff from Ada to Bob, sent with `tools`, whose turn has raised a
/// card and waits at its gate to end.
pub(super) async fn asking(
    room: &Arc<Room>,
    tools: TeammateTools,
    bob: &Fake,
    task: &str,
) -> (String, String) {
    let sent = tools
        .call(
            "message_teammate",
            &json!({"to":"bob","message":task,"intent":"handoff"}),
        )
        .await
        .unwrap();
    let handoff = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("bob's turn is running", || !bob.prompts().is_empty()).await;
    let thread = work_thread(room, &handoff);
    TeammateTools::new(room, "bob")
        .for_work(thread.clone())
        .call(
            "request_human",
            &json!({"reason": format!("Approve {task}")}),
        )
        .await
        .unwrap();
    (handoff, thread)
}

/// Whether a turn holds Ada's DM, or has just been handed it.
pub(super) fn claimed(room: &Room) -> bool {
    room.session("ada")
        .is_ok_and(|session| lock(&session.turns).running)
}

/// How many turns a report has woken.
fn woken(agents: &Fake) -> usize {
    agents
        .prompts()
        .iter()
        .filter(|prompt| prompt.ends_with(WAKE_LINE))
        .count()
}

fn exchanges(room: &Room) -> i64 {
    room.exchange_pair("ada~bob").unwrap().exchanges
}

fn paused_cards(room: &Room, whose: &str) -> Vec<serde_json::Value> {
    room.tape(whose)
        .into_iter()
        .filter(|v| v["kind"] == "exchange_paused")
        .collect()
}

/// What the teammate said in a thread, in order.
fn said_in(room: &Room, thread: &str) -> Vec<String> {
    room.log
        .load(&StreamId::Side(thread.to_string()))
        .into_iter()
        .filter(|v| v["kind"] == "agent")
        .map(|v| v["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The reports shown in a work thread, as the person reads them.
fn shown_in(room: &Room, thread: &str) -> Vec<String> {
    room.log
        .load(&StreamId::Side(thread.to_string()))
        .into_iter()
        .filter(|v| {
            v["kind"] == "notice"
                && v["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("progress:"))
        })
        .map(|v| v["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The first line a driver heard that ends with `line`.
pub(super) fn heard_with(agents: &Fake, line: &str) -> Option<String> {
    agents.prompts().into_iter().find(|p| p.ends_with(line))
}

/// A handoff from Ada to Bob that has returned its result, with Ada's turn
/// on that result over and Bob's thread closed by it.
async fn handed_over_and_done(name: &str) -> (Arc<Room>, Arc<Fake>, String, String) {
    let (room, agents) = setup(name);
    room.allow_sender("bob", "ada").unwrap();
    let handoff = send(&room, "handoff").await;
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    until(
        "ada has heard the result and bob's thread has closed",
        || agents.prompts().len() == 2 && !room.mid_turn("ada") && !room.work_is_open(&thread),
    )
    .await;
    (room, agents, handoff, thread)
}

#[tokio::test]
async fn the_persons_turn_in_a_delegated_thread_reaches_the_delegator_as_one_report_without_their_words()
 {
    let (room, agents, handoff, thread) = handed_over_and_done("progress-persons-turn").await;
    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "my secret is swordfish", None)
        .await
        .unwrap();
    until("the report waits for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 1
    })
    .await;
    let report = said_in(&room, &thread).last().unwrap().clone();
    let shown = notices(&room, "ada");
    assert_eq!(
        shown,
        [format!(
            "Ada, in your handoff thread “work handoff”: {report}"
        )]
    );
    let saved = request(&room, "ada~bob", &handoff);
    assert_eq!(saved.reply, "result-0", "the handoff's result is unchanged");
    assert!(!saved.progress_wakes, "the person's turn wakes nobody");

    room.prompt("ada", "next", None, None).await.unwrap();
    until("ada hears her line", || {
        heard_with(&agents, "next").is_some()
    })
    .await;
    let heard = heard_with(&agents, "next").unwrap();
    assert_eq!(heard.matches(&report).count(), 1, "{heard}");
    assert!(heard.contains("in the thread you handed them"), "{heard}");
    assert!(!heard.contains("swordfish"), "{heard}");
    assert!(
        room.exchange_pair("ada~bob")
            .unwrap()
            .requests
            .iter()
            .all(|r| r.progress.is_empty() && r.progress_left_out == 0 && !r.progress_wakes)
    );

    room.prompt("ada", "and again", None, None).await.unwrap();
    until("ada hears her second line", || {
        heard_with(&agents, "and again").is_some()
    })
    .await;
    assert!(!heard_with(&agents, "and again").unwrap().contains(&report));
}

#[tokio::test]
async fn a_handoff_turn_that_stops_for_the_person_reports_the_cards_reason() {
    let (room, handoff, _action) = suspended_handoff("progress-card").await;
    until("ada is told", || !notices(&room, "ada").is_empty()).await;
    assert_eq!(
        notices(&room, "ada"),
        [
            "Ada, in your handoff thread “work handoff”: Waiting for the person\n\nWaiting on the person: Approve the deployment"
        ]
    );
    let saved = request(&room, "ada~bob", &handoff);
    assert_eq!(saved.progress.len(), 1);
    assert!(
        !saved.progress_wakes,
        "ada is not running, so it waits for what she hears next"
    );
}

#[tokio::test]
async fn a_handoff_turn_waiting_on_another_colleague_reports_it() {
    let handed = handed_off("progress-colleague", "end_turn").await;
    let room = &handed.room;
    ask_carol(room, &handed.thread).await;
    handed.bob_gate.add_permits(2);
    until("ada is told", || !notices(room, "ada").is_empty()).await;
    let shown = notices(room, "ada");
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(
        shown[0].ends_with(": I asked Carol which colour.\n\nWaiting on a colleague's answer."),
        "{shown:?}"
    );
}

/// Bob may not ask Carol yet: his question waits on the person's card in his
/// main conversation, and the handoff's report says so.
#[tokio::test]
async fn a_handoff_turn_waiting_on_a_question_the_person_must_allow_reports_the_card() {
    let log = scratch("progress-collaboration-card");
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
        Scripted::new(turn("asked", "I asked Carol which colour.", "end_turn")).gated(gate.clone()),
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(super::gates::Apart {
            bob: bob.clone(),
            others: Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn"))),
        }),
    );
    room.allow_sender("bob", "ada").unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    ask_carol(&room, &thread).await;
    gate.add_permits(2);
    until("ada is told", || !notices(&room, "ada").is_empty()).await;
    assert_eq!(
        notices(&room, "ada"),
        [
            "Bob, in your handoff thread “work handoff”: I asked Carol which colour.\n\nWaiting on the person: Allow Bob to ask Carol to work? The card is in Bob's main conversation."
        ]
    );
    assert_eq!(request(&room, "ada~bob", &handoff).progress.len(), 1);
    until("the card is up in Bob's main conversation", || {
        open_card(&room, "bob").is_some()
    })
    .await;
}

/// Bob and Carol have paused at twelve: Bob's question waits on Keep going,
/// whatever his grant, and the report names the card that is up, not one
/// for allowing him to ask that only comes once they go on.
#[tokio::test]
async fn a_handoff_turn_waiting_on_a_question_its_paused_pair_holds_reports_the_pause() {
    for granted in [false, true] {
        let log = scratch(&format!("progress-paused-question-{granted}"));
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
            Scripted::new(turn("asked", "I asked Carol which colour.", "end_turn"))
                .gated(gate.clone()),
        );
        let room = Room::with_agents(
            log,
            Arc::new(DeskKeys),
            Arc::new(super::gates::Apart {
                bob: bob.clone(),
                others: Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn"))),
            }),
        );
        room.allow_sender("bob", "ada").unwrap();
        if granted {
            room.allow_sender("carol", "bob").unwrap();
        }
        let paused = Pair {
            id: thread_key("bob", "carol").unwrap(),
            a: "bob".into(),
            b: "carol".into(),
            exchanges: EXCHANGE_CAP,
            paused: true,
            requests: vec![],
        };
        room.save_pair(&paused).unwrap();
        room.exchange_card(&paused, ExchangePauseStatus::Pending);
        let handoff = send(&room, "handoff").await;
        until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
        let thread = work_thread(&room, &handoff);
        ask_carol(&room, &thread).await;
        gate.add_permits(2);
        until("ada is told", || !notices(&room, "ada").is_empty()).await;
        assert_eq!(
            notices(&room, "ada"),
            [
                "Bob, in your handoff thread “work handoff”: I asked Carol which colour.\n\nWaiting on the person: Let Bob and Carol keep going? They paused at 12 messages. The card is in Bob's main conversation."
            ],
            "{granted}"
        );
        assert!(open_card(&room, "bob").is_none(), "{granted}");
        assert_eq!(paused_cards(&room, "bob")[0]["status"], "pending");
    }
}

/// The person's Stop on a handoff's own turn ends the handoff. Its sender
/// hears that once, as the person's decision, with whatever it hears next:
/// nothing wakes it, and nothing asks it to inspect and send the work again.
/// The thread closes as stopped, and can be continued.
#[tokio::test]
async fn the_persons_stop_on_a_handoffs_turn_ends_it_and_waits_for_the_senders_next_line() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("started", "Running the long command.", "end_turn"))
            .on_cancel(vec![Update::Turn {
                stop_reason: "cancelled".into(),
                usage: None,
            }])
            .gated(gate.clone()),
    );
    let room = room_with("progress-person-stops-handoff", &ada, vec![bob.clone()]);
    // Ada is up and idle, so a report that woke her would start a turn.
    room.start("ada").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || bob.prompts().len() == 1).await;
    let thread = work_thread(&room, &handoff);
    gate.add_permits(1);
    until("bob has said what he is doing", || {
        said_in(&room, &thread).len() == 1
    })
    .await;
    assert!(!claimed(&room), "ada is idle");
    room.cancel_side(&thread).unwrap();
    until("the thread has closed", || !room.work_is_open(&thread)).await;

    let stopped = request(&room, "ada~bob", &handoff);
    assert_eq!((stopped.phase, stopped.failed), (Phase::Stopped, true));
    let line = "The person stopped this handoff. Do not send it again unless they ask.";
    assert_eq!(
        notices(&room, "ada"),
        [format!(
            "Ada, in your handoff thread “work handoff”: Running the long command.\n\n{line}"
        )]
    );
    assert!(!stopped.progress_wakes);
    assert_eq!(
        room.link_of(&ThreadId::side(&thread)).unwrap().state,
        crate::thread::ThreadState::Closed(End::Stopped)
    );
    assert_eq!(says_ended(&room, &thread), [true; 3]);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        room.tape("ada")
            .iter()
            .all(|v| v["kind"] != "delivery" || v["cause"]["requestId"] != handoff.as_str()),
        "no result is delivered for it"
    );
    assert!(ada.prompts().is_empty(), "nothing woke ada");
    assert!(!claimed(&room), "nor is a turn of hers under way");

    room.prompt("ada", "what happened?", None, None)
        .await
        .unwrap();
    until("ada hears her line", || {
        heard_with(&ada, "what happened?").is_some()
    })
    .await;
    let heard = heard_with(&ada, "what happened?").unwrap();
    assert_eq!(heard.matches(line).count(), 1, "{heard}");
    assert!(!heard.contains("inspect before retrying"), "{heard}");
    room.prompt("ada", "and now?", None, None).await.unwrap();
    until("ada hears her second line", || {
        heard_with(&ada, "and now?").is_some()
    })
    .await;
    assert!(!heard_with(&ada, "and now?").unwrap().contains(line));
    room.continue_side(&thread).await.unwrap();
}

/// The question reaches the teammate that handed the thread over by itself,
/// so a report of the wait would say it twice.
#[tokio::test]
async fn a_handoff_turn_waiting_only_on_its_sender_reports_nothing() {
    let handed = handed_off("progress-asks-its-sender", "end_turn").await;
    let room = &handed.room;
    ask_from(room, &handed.thread, "ada").await;
    handed.bob_gate.add_permits(2);
    until("the handoff waits on ada and its turn has ended", || {
        request(room, "ada~bob", &handed.handoff).phase == Phase::Waiting
            && idle(room, &handed.thread)
    })
    .await;
    assert!(notices(room, "ada").is_empty());
    assert!(
        request(room, "ada~bob", &handed.handoff)
            .progress
            .is_empty()
    );
}

#[tokio::test]
async fn the_turn_that_returns_the_result_reports_nothing() {
    let (room, _agents, _handoff, _thread) = handed_over_and_done("progress-result").await;
    assert!(notices(&room, "ada").is_empty());
    assert!(
        room.exchange_pair("ada~bob")
            .unwrap()
            .requests
            .iter()
            .all(|r| r.progress.is_empty())
    );
}

#[tokio::test]
async fn reports_heard_late_are_one_summary_per_thread_and_survive_a_restart() {
    let (room, agents, handoff, thread) = handed_over_and_done("progress-late").await;
    room.continue_side(&thread).await.unwrap();
    for line in ["one", "two", "three"] {
        room.prompt_side(&thread, line, None).await.unwrap();
    }
    until("three reports wait for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 3
    })
    .await;
    assert_eq!(notices(&room, "ada").len(), 3);
    let first = said_in(&room, &thread)[1..].join("\n---\n");
    room.prompt("ada", "what happened?", None, None)
        .await
        .unwrap();
    until("ada hears her line", || {
        heard_with(&agents, "what happened?").is_some()
    })
    .await;
    let heard = heard_with(&agents, "what happened?").unwrap();
    assert_eq!(heard.matches("in the thread you handed them").count(), 1);
    assert!(
        heard.contains(&format!(
            "Ada, in the thread you handed them (side:{thread}):\n{first}\n"
        )),
        "{heard}"
    );

    for line in ["four", "five", "six"] {
        room.prompt_side(&thread, line, None).await.unwrap();
    }
    until("three more reports wait for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 3
    })
    .await;
    let later = said_in(&room, &thread)[4..].join("\n---\n");
    let log = room.log.clone();
    drop(room);
    let agents = Fake::new(Scripted::new(vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "heard".into(),
            text: "Heard.".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "back again", None, None).await.unwrap();
    until("ada hears her line after the restart", || {
        heard_with(&agents, "back again").is_some()
    })
    .await;
    let heard = heard_with(&agents, "back again").unwrap();
    assert!(
        heard.contains(&format!(
            "Ada, in the thread you handed them (side:{thread}):\n{later}\n"
        )),
        "{heard}"
    );
    assert!(!heard.contains(&first), "{heard}");
    room.prompt("ada", "once more", None, None).await.unwrap();
    until("ada hears her second line", || {
        heard_with(&agents, "once more").is_some()
    })
    .await;
    assert!(!heard_with(&agents, "once more").unwrap().contains(&later));
}

#[tokio::test]
async fn a_thread_reporting_past_the_limit_drops_the_oldest_and_says_how_many() {
    let (room, _agents, _handoff, thread) = handed_over_and_done("progress-limit").await;
    let reports: Vec<String> = (0..10)
        .map(|i| format!("{i:02}{}", "x".repeat(298)))
        .collect();
    for report in &reports {
        room.report_progress(&thread, report.clone(), false);
    }
    assert_eq!(notices(&room, "ada").len(), 10, "each is shown at once");
    let heard = room.take_heard(&ThreadId::dm("ada")).unwrap();
    assert!(
        heard.contains(&format!(
            "(side:{thread}):\n{}\n(5 earlier reports were left out.)",
            reports[5..].join("\n---\n")
        )),
        "{heard}"
    );
    assert!(reports[..5].iter().all(|report| !heard.contains(report)));
    assert_eq!(room.take_heard(&ThreadId::dm("ada")), None);
}

/// Where a report is heard is worked out when it is heard.
#[tokio::test]
async fn a_report_is_heard_in_the_work_thread_it_was_handed_from_or_in_the_dm_once_that_closed() {
    let (room, agents) = setup("progress-from-a-work-thread");
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
    let sent = TeammateTools::new(&room, "ada")
        .for_work(work.clone())
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"work handoff","intent":"handoff"}),
        )
        .await
        .unwrap();
    let handoff = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    until("ada's work thread has heard the result", || {
        agents.prompts().len() == 3 && idle(&room, &work) && !room.work_is_open(&thread)
    })
    .await;

    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "make it shorter", None)
        .await
        .unwrap();
    until("the report waits for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 1
    })
    .await;
    let report = said_in(&room, &thread).last().unwrap().clone();
    let shown = shown_in(&room, &work);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].ends_with(&report));
    assert!(notices(&room, "ada").is_empty());
    room.prompt_side(&work, "what came back?", None)
        .await
        .unwrap();
    until("ada's work thread hears its line", || {
        heard_with(&agents, "what came back?").is_some()
    })
    .await;
    assert!(
        heard_with(&agents, "what came back?")
            .unwrap()
            .contains(&report)
    );

    // Shown in the work thread while it is open, then the thread closes
    // before it takes another line: the DM is where it is heard.
    until("ada's work thread is idle", || idle(&room, &work)).await;
    room.prompt_side(&thread, "and the summary?", None)
        .await
        .unwrap();
    until("a second report waits for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 1
    })
    .await;
    let late = said_in(&room, &thread).last().unwrap().clone();
    let shown = shown_in(&room, &work);
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(shown[1].ends_with(&late));
    room.archive_side(&work, SideEnd::Person, None).unwrap();
    room.start("ada").await.unwrap();
    room.prompt("ada", "anything new?", None, None)
        .await
        .unwrap();
    until("ada hears her line", || {
        heard_with(&agents, "anything new?").is_some()
    })
    .await;
    let heard = heard_with(&agents, "anything new?").unwrap();
    assert_eq!(heard.matches(&late).count(), 1, "{heard}");
    assert!(!heard.contains(&report), "{heard}");
    assert!(notices(&room, "ada").is_empty(), "it was shown once, in W");
    assert!(request(&room, "ada~bob", &handoff).progress.is_empty());

    // Made once W has closed, a report is shown in the DM and heard there.
    room.prompt_side(&thread, "and the date?", None)
        .await
        .unwrap();
    until("a third report waits for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 1
    })
    .await;
    let after = said_in(&room, &thread).last().unwrap().clone();
    let shown = notices(&room, "ada");
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].ends_with(&after));
    assert_eq!(shown_in(&room, &work).len(), 2);
    room.prompt("ada", "and now?", None, None).await.unwrap();
    until("ada hears her line", || {
        heard_with(&agents, "and now?").is_some()
    })
    .await;
    let heard = heard_with(&agents, "and now?").unwrap();
    assert_eq!(heard.matches(&after).count(), 1, "{heard}");

    room.continue_side(&work).await.unwrap();
    room.prompt_side(&work, "still there?", None).await.unwrap();
    until("ada's work thread hears its line", || {
        heard_with(&agents, "still there?").is_some()
    })
    .await;
    let heard = heard_with(&agents, "still there?").unwrap();
    assert!(!heard.contains(&late) && !heard.contains(&after), "{heard}");
}

/// A report is cut by characters, not bytes, and a turn that said nothing
/// and ended well has nothing to report.
#[test]
fn a_report_is_cut_by_characters_and_an_empty_one_is_not_sent() {
    let long = Driven {
        last_words: vec!["é".repeat(500)],
        ..Driven::default()
    };
    let report = turn_report(&long, TurnEnd::Said).unwrap();
    assert_eq!(report.chars().count(), REPORT_CHARS + 1);
    assert!(report.ends_with('…'));
    assert_eq!(turn_report(&Driven::default(), TurnEnd::Said), None);
    assert_eq!(
        turn_report(&Driven::default(), TurnEnd::Stopped).as_deref(),
        Some("This turn was stopped.")
    );
    let ended_on_a_tool = Driven {
        replies: vec!["Looking.".into(), "Running the build.".into()],
        ..Driven::default()
    };
    assert_eq!(
        turn_report(&ended_on_a_tool, TurnEnd::Failed).as_deref(),
        Some("Running the build.\n\nThis turn failed.")
    );
}

/// Threads that end together wake the teammate that handed them over once,
/// with every report, once none of them is still at work.
#[tokio::test]
async fn a_report_waits_while_another_handed_over_thread_works_and_wakes_once_with_both() {
    let ada = Fake::new(Scripted::new(turn("heard", "On it.", "end_turn")));
    let gates = [Arc::new(Semaphore::new(0)), Arc::new(Semaphore::new(0))];
    let bobs: Vec<Arc<Fake>> = gates
        .iter()
        .map(|gate| {
            Fake::new(
                Scripted::new(turn("waiting", "Waiting for the person.", "end_turn"))
                    .gated(gate.clone()),
            )
        })
        .collect();
    let room = room_with("progress-wakes-once", &ada, bobs.clone());
    room.start("ada").await.unwrap();
    let mut handed = Vec::new();
    for (bob, task) in bobs.iter().zip(["paint the shed", "fix the gate"]) {
        handed.push(asking(&room, TeammateTools::new(&room, "ada"), bob, task).await);
    }
    let counted = exchanges(&room);

    gates[0].add_permits(2);
    until("the first thread waits on its card, its turn over", || {
        request(&room, "ada~bob", &handed[0].0).phase == Phase::Waiting && idle(&room, &handed[0].1)
    })
    .await;
    assert_eq!(notices(&room, "ada").len(), 1);
    assert!(!claimed(&room), "the other thread is still at work");

    gates[1].add_permits(2);
    until("ada is woken and her turn is over", || {
        woken(&ada) == 1 && !claimed(&room)
    })
    .await;
    let prompts = ada.prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    for (_, thread) in &handed {
        assert!(
            prompts[0].contains(&format!("(side:{thread})")),
            "{}",
            prompts[0]
        );
    }
    assert_eq!(notices(&room, "ada").len(), 2);
    assert_eq!(exchanges(&room), counted + 1, "the wake is one exchange");
}

/// The person's turn, a stopped one and a failed one are shown at once and
/// wait for what the delegator hears next.
#[tokio::test]
async fn a_persons_stopped_or_failed_turn_waits_for_the_delegators_next_line() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let handing =
        Fake::new(Scripted::new(turn("result", "Handed back.", "end_turn")).gated(gate.clone()));
    // An agent of its own once the thread is continued: the first one is
    // stopped as the thread closes, and that stop would cut its next turn.
    let bob = Fake::new(
        Scripted::turns(vec![
            turn("plain", "Renamed the file.", "end_turn"),
            turn("cut", "Starting the migration.", "end_turn"),
            turn("broke", "The build broke.", "failed"),
        ])
        .on_cancel(vec![Update::Turn {
            stop_reason: "cancelled".into(),
            usage: None,
        }])
        .gated(gate.clone()),
    );
    let room = room_with("progress-waits", &ada, vec![handing, bob.clone()]);
    let handoff = send(&room, "handoff").await;
    gate.add_permits(2);
    done(&room, &handoff).await;
    let thread = work_thread(&room, &handoff);
    until(
        "ada has heard the result and bob's thread has closed",
        || ada.prompts().len() == 1 && !claimed(&room) && !room.work_is_open(&thread),
    )
    .await;
    let reports = || request(&room, "ada~bob", &handoff).progress.len();
    room.continue_side(&thread).await.unwrap();

    room.prompt_side(&thread, "rename it", None).await.unwrap();
    gate.add_permits(2);
    until("the person's turn is reported", || reports() == 1).await;
    room.prompt_side(&thread, "migrate it", None).await.unwrap();
    until("bob's turn is running", || bob.prompts().len() == 2).await;
    room.cancel_side(&thread).unwrap();
    until("the stopped turn is reported", || reports() == 2).await;
    room.prompt_side(&thread, "build it", None).await.unwrap();
    gate.add_permits(2);
    until("the failed turn is reported", || reports() == 3).await;

    assert_eq!(notices(&room, "ada").len(), 3);
    assert!(!claimed(&room));
    assert_eq!(ada.prompts().len(), 1, "nothing woke ada");
    room.prompt("ada", "what happened?", None, None)
        .await
        .unwrap();
    until("ada hears her line", || {
        heard_with(&ada, "what happened?").is_some()
    })
    .await;
    let heard = heard_with(&ada, "what happened?").unwrap();
    for report in [
        "Renamed the file.",
        "This turn was stopped.",
        "The build broke.\n\nThis turn failed.",
    ] {
        assert!(heard.contains(report), "{heard}");
    }
    assert_eq!(woken(&ada), 0);
}

#[tokio::test]
async fn a_busy_delegator_is_not_woken_and_hears_the_report_with_its_next_line() {
    let ada_gate = Arc::new(Semaphore::new(0));
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")).gated(ada_gate.clone()));
    let bob_gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(
        Scripted::new(turn("waiting", "Waiting for the person.", "end_turn"))
            .gated(bob_gate.clone()),
    );
    let room = room_with("progress-busy", &ada, vec![bob.clone()]);
    room.start("ada").await.unwrap();
    let (handoff, thread) = asking(
        &room,
        TeammateTools::new(&room, "ada"),
        &bob,
        "paint the shed",
    )
    .await;
    room.prompt("ada", "busy?", None, None).await.unwrap();
    until("ada's turn is running", || ada.prompts().len() == 1).await;

    bob_gate.add_permits(2);
    until("the report waits for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 1 && idle(&room, &thread)
    })
    .await;
    assert!(!request(&room, "ada~bob", &handoff).progress_wakes);
    assert_eq!(notices(&room, "ada").len(), 1);
    assert_eq!(ada.prompts().len(), 1, "a busy delegator is not woken");
    ada_gate.add_permits(2);
    until("ada's turn is over", || !claimed(&room)).await;
    assert_eq!(ada.prompts().len(), 1, "nor when her turn ends");

    room.prompt("ada", "next", None, None).await.unwrap();
    ada_gate.add_permits(2);
    until("ada hears her next line", || {
        heard_with(&ada, "next").is_some()
    })
    .await;
    let heard = heard_with(&ada, "next").unwrap();
    assert!(
        heard.contains("Waiting on the person: Approve paint the shed"),
        "{heard}"
    );
    assert_eq!(woken(&ada), 0);
}

/// Each wake is an automatic exchange of the pair's twelve: once the pair
/// pauses, reports only wait, until the person speaks.
#[tokio::test]
async fn wakes_count_toward_the_pairs_twelve_and_then_reports_only_wait() {
    let (room, agents, handoff, thread) = handed_over_and_done("progress-wakes-count").await;
    let mut pair = room.exchange_pair("ada~bob").unwrap();
    pair.exchanges = 10;
    room.save_pair(&pair).unwrap();
    for (woke, count) in [(1, 11), (2, 12)] {
        room.report_progress(&thread, format!("report {woke}"), true);
        until("ada is woken and her turn is over", || {
            woken(&agents) == woke && !claimed(&room)
        })
        .await;
        assert_eq!(exchanges(&room), count);
    }
    assert!(room.exchange_pair("ada~bob").unwrap().paused);
    for whose in ["ada", "bob"] {
        let cards = paused_cards(&room, whose);
        assert_eq!(cards.len(), 1, "{whose}: {cards:?}");
        assert_eq!(cards[0]["status"], "pending");
    }

    room.report_progress(&thread, "report 3".into(), true);
    assert!(!claimed(&room), "a paused pair wakes nobody");
    assert_eq!(woken(&agents), 2);
    assert!(notices(&room, "ada").last().unwrap().ends_with("report 3"));
    assert!(!request(&room, "ada~bob", &handoff).progress_wakes);

    room.prompt("ada", "hi", None, None).await.unwrap();
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.exchanges, 0);
    assert!(!pair.paused);
    for whose in ["ada", "bob"] {
        assert_eq!(paused_cards(&room, whose)[0]["status"], "resumed");
    }
    until("ada hears her line", || heard_with(&agents, "hi").is_some()).await;
    assert!(heard_with(&agents, "hi").unwrap().contains("report 3"));
}

/// A wake whose reports another line heard first is no turn: dispatched
/// with the person's line, a report is heard once, and a wake that lost
/// that race waits behind the line that heard it.
#[tokio::test]
async fn a_wake_that_finds_its_reports_heard_is_skipped() {
    let (room, agents, _handoff, thread) = handed_over_and_done("progress-wake-skipped").await;
    let (said, ()) = tokio::join!(room.prompt("ada", "hi", None, None), async {
        room.report_progress(&thread, "Swatch 41 is ready.".into(), true);
    });
    said.unwrap();
    until("ada has heard her line and is idle", || {
        heard_with(&agents, "hi").is_some() && !claimed(&room)
    })
    .await;
    let carried = agents
        .prompts()
        .iter()
        .filter(|prompt| prompt.contains("Swatch 41 is ready."))
        .count();
    assert_eq!(carried, 1);

    room.report_progress(&thread, "Swatch 42 is ready.".into(), false);
    room.prompt("ada", "and now?", None, None).await.unwrap();
    room.dispatch(
        room.session("ada").unwrap(),
        Wired {
            wake: true,
            ..Wired::words(timed(now_ms(), WAKE_LINE))
        },
    );
    until("ada has heard her line and is idle", || {
        heard_with(&agents, "and now?").is_some() && !claimed(&room)
    })
    .await;
    assert!(
        heard_with(&agents, "and now?")
            .unwrap()
            .contains("Swatch 42 is ready.")
    );
    let prompts = agents.prompts();
    assert!(
        prompts.iter().all(|prompt| prompt != WAKE_LINE),
        "{prompts:?}"
    );
}

/// The job a handoff is sent from: a loop not due for an hour.
pub(super) fn planted_job(room: &Room, id: &str) -> ScheduledJob {
    let now = now_ms();
    let job = ScheduledJob {
        id: id.to_string(),
        persona_id: "ada".to_string(),
        kind: ScheduleKind::Loop,
        when: None,
        every: Some(3_600_000),
        prompt: "watch the build".to_string(),
        quiet: None,
        reminder: None,
        goal: None,
        until: None,
        times: None,
        without: None,
        operator_created: true,
        next_at: now + 3_600_000,
        created_at: now,
        last_post: None,
        ended_at: None,
    };
    crate::room::append_schedule(&room.log, &job).unwrap();
    job
}

/// A report wakes the work thread or the job it was handed from, when that
/// is where it is heard and is idle.
#[tokio::test]
async fn a_report_wakes_the_idle_work_thread_or_job_it_was_handed_from() {
    for from in ["work", "job"] {
        let ada = Fake::new(Scripted::new(turn("heard", "On it.", "end_turn")));
        let gate = Arc::new(Semaphore::new(0));
        let bob = Fake::new(
            Scripted::new(turn("waiting", "Waiting for the person.", "end_turn"))
                .gated(gate.clone()),
        );
        let room = room_with(&format!("progress-wakes-a-{from}"), &ada, vec![bob.clone()]);
        let (tools, conversation) = if from == "work" {
            let work = room
                .start_side("ada", "plan the release")
                .await
                .unwrap()
                .side_id;
            (
                TeammateTools::new(&room, "ada").for_work(work.clone()),
                ThreadId::side(&work),
            )
        } else {
            let job = planted_job(&room, "job-build");
            room.prompt_job(&job.id, "watch it", None).await.unwrap();
            (
                TeammateTools::new(&room, "ada")
                    .with_capability(room.capability_lease("ada"))
                    .for_job(job.id.clone()),
                ThreadId::job(&job.id),
            )
        };
        let rests = || {
            if from == "work" {
                idle(&room, &conversation.key)
            } else {
                room.job_idle(&conversation.key)
            }
        };
        until("ada's thread has had its turn", || {
            ada.prompts().len() == 1 && rests()
        })
        .await;
        let (_handoff, thread) = asking(&room, tools, &bob, "paint the shed").await;

        gate.add_permits(2);
        until("ada's thread is woken and rests again", || {
            woken(&ada) == 1 && rests()
        })
        .await;
        let woke = ada.prompts().pop().unwrap();
        assert!(woke.contains(&format!("(side:{thread})")), "{woke}");
        assert!(notices(&room, "ada").is_empty(), "{from}");
        assert!(!claimed(&room), "{from}");
        let shown = room
            .log
            .load(&StreamId::of_thread(&conversation).unwrap())
            .into_iter()
            .filter(|v| {
                v["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("progress:"))
            })
            .count();
        assert_eq!(shown, 1, "{from}");
    }
}

/// A report held back while another handed-over thread was at work wakes the
/// delegator when that thread rests, even after a turn that wakes nobody.
#[tokio::test]
async fn a_held_back_report_wakes_the_delegator_when_the_other_thread_rests_after_a_non_waking_turn()
 {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let (gate_one, gate_two) = (Arc::new(Semaphore::new(0)), Arc::new(Semaphore::new(0)));
    let handing = Fake::new(Scripted::new(turn("fixed", "Fixed the gate.", "end_turn")));
    let continued =
        Fake::new(Scripted::new(turn("tidied", "Tidied up.", "end_turn")).gated(gate_two.clone()));
    let waiting = Fake::new(
        Scripted::new(turn("waiting", "Waiting for the person.", "end_turn"))
            .gated(gate_one.clone()),
    );
    let room = room_with(
        "progress-held-back",
        &ada,
        vec![handing, continued.clone(), waiting.clone()],
    );
    let done_handoff = hand_off(&room, "ada", "bob", "fix the gate", None)
        .await
        .unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    done(&room, &done_handoff).await;
    let two = work_thread(&room, &done_handoff);
    until("ada has heard the result and its thread has closed", || {
        ada.prompts().len() == 1 && !claimed(&room) && !room.work_is_open(&two)
    })
    .await;
    room.continue_side(&two).await.unwrap();
    room.prompt_side(&two, "tidy up", None).await.unwrap();
    until("the person's turn is running", || {
        continued.prompts().len() == 1
    })
    .await;
    let (handoff, one) = asking(
        &room,
        TeammateTools::new(&room, "ada"),
        &waiting,
        "paint the shed",
    )
    .await;

    gate_one.add_permits(2);
    until("the handoff waits on its card, its thread at rest", || {
        request(&room, "ada~bob", &handoff).phase == Phase::Waiting && idle(&room, &one)
    })
    .await;
    assert_eq!(notices(&room, "ada").len(), 1);
    assert!(!claimed(&room), "the person's turn is still at work");
    let counted = exchanges(&room);

    gate_two.add_permits(2);
    until("ada is woken and her turn is over", || {
        woken(&ada) == 1 && !claimed(&room)
    })
    .await;
    let woke = ada.prompts().pop().unwrap();
    for thread in [&one, &two] {
        assert!(woke.contains(&format!("(side:{thread})")), "{woke}");
    }
    assert!(woke.contains("Tidied up."), "{woke}");
    assert_eq!(notices(&room, "ada").len(), 2);
    assert_eq!(ada.prompts().len(), 2, "woken once");
    assert_eq!(exchanges(&room), counted + 1);
}

/// A report that came while its delegator was busy waits for her next line:
/// another handed-over thread resting later does not wake her for it.
#[tokio::test]
async fn a_report_turned_down_while_the_delegator_was_busy_is_not_woken_by_a_later_rest() {
    let ada_gate = Arc::new(Semaphore::new(0));
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")).gated(ada_gate.clone()));
    let (gate_one, gate_two) = (Arc::new(Semaphore::new(0)), Arc::new(Semaphore::new(0)));
    let handing = Fake::new(Scripted::new(turn("fixed", "Fixed the gate.", "end_turn")));
    let continued =
        Fake::new(Scripted::new(turn("tidied", "Tidied up.", "end_turn")).gated(gate_two.clone()));
    let waiting = Fake::new(
        Scripted::new(turn("waiting", "Waiting for the person.", "end_turn"))
            .gated(gate_one.clone()),
    );
    let room = room_with(
        "progress-busy-then-rest",
        &ada,
        vec![handing, continued.clone(), waiting.clone()],
    );
    let done_handoff = hand_off(&room, "ada", "bob", "fix the gate", None)
        .await
        .unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    ada_gate.add_permits(2);
    done(&room, &done_handoff).await;
    let two = work_thread(&room, &done_handoff);
    until("ada has heard the result and its thread has closed", || {
        ada.prompts().len() == 1 && !claimed(&room) && !room.work_is_open(&two)
    })
    .await;
    room.continue_side(&two).await.unwrap();
    let (handoff, one) = asking(
        &room,
        TeammateTools::new(&room, "ada"),
        &waiting,
        "paint the shed",
    )
    .await;
    room.prompt("ada", "busy?", None, None).await.unwrap();
    until("ada's turn is running", || ada.prompts().len() == 2).await;

    gate_one.add_permits(2);
    until("the handoff waits on its card, its thread at rest", || {
        request(&room, "ada~bob", &handoff).phase == Phase::Waiting && idle(&room, &one)
    })
    .await;
    ada_gate.add_permits(2);
    until("ada's turn is over", || !claimed(&room)).await;

    room.prompt_side(&two, "tidy up", None).await.unwrap();
    gate_two.add_permits(2);
    until("the person's turn is reported and its thread rests", || {
        notices(&room, "ada").len() == 2 && idle(&room, &two)
    })
    .await;
    assert!(!claimed(&room), "nothing wakes ada");
    assert_eq!(ada.prompts().len(), 2);

    room.prompt("ada", "next", None, None).await.unwrap();
    ada_gate.add_permits(2);
    until("ada hears her next line", || {
        heard_with(&ada, "next").is_some()
    })
    .await;
    let heard = heard_with(&ada, "next").unwrap();
    for thread in [&one, &two] {
        assert!(heard.contains(&format!("(side:{thread})")), "{heard}");
    }
    assert!(heard.contains("Tidied up."), "{heard}");
    assert_eq!(woken(&ada), 0);
}

/// A room closing for a restart starts no new turn for a report: nothing is
/// woken or counted, and the report stays with the handoff.
#[tokio::test]
async fn a_closing_room_wakes_nobody_and_keeps_the_report() {
    let (room, agents, handoff, thread) = handed_over_and_done("progress-closing").await;
    let counted = exchanges(&room);
    room.closing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    room.report_progress(&thread, "Swatch 43 is ready.".into(), true);
    assert!(!claimed(&room), "a closing room wakes nobody");
    assert_eq!(woken(&agents), 0);
    assert_eq!(exchanges(&room), counted);
    let kept = request(&room, "ada~bob", &handoff);
    assert_eq!(kept.progress, ["Swatch 43 is ready."]);
}

/// A done handoff whose thread the person has talked in three times: Ada has
/// three reports she has not heard.
async fn with_unheard_reports(name: &str) -> (Arc<Room>, String, String) {
    let (room, _agents, handoff, thread) = handed_over_and_done(name).await;
    room.continue_side(&thread).await.unwrap();
    for line in ["one", "two", "three"] {
        room.prompt_side(&thread, line, None).await.unwrap();
    }
    until("three reports wait for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 3
    })
    .await;
    (room, handoff, thread)
}

/// What the person's turns in the thread have written to a conversation:
/// the reports shown there, whoever they are for.
fn reports_shown(room: &Room, thread: &str) -> usize {
    notices(room, "ada").len() + notices(room, "bob").len() + shown_in(room, thread).len()
}

/// The person talks in the thread, and that turn reports to nobody.
async fn talks_without_a_report(room: &Arc<Room>, handoff: &str, thread: &str) {
    let (shown, said) = (reports_shown(room, thread), said_in(room, thread).len());
    room.continue_side(thread).await.unwrap();
    room.prompt_side(thread, "and the date?", None)
        .await
        .unwrap();
    until("bob's turn is over", || {
        said_in(room, thread).len() > said && idle(room, thread)
    })
    .await;
    assert_eq!(reports_shown(room, thread), shown, "no report is shown");
    let saved = request(room, "ada~bob", handoff);
    assert!(
        saved.progress.is_empty() && !saved.progress_wakes,
        "nor kept"
    );
}

/// Every request of the pair has nothing for its sender to hear, and the
/// handoff no longer reports.
fn is_released(room: &Room, handoff: &str) {
    for saved in room.exchange_pair("ada~bob").unwrap().requests {
        assert!(
            saved.progress.is_empty() && saved.progress_left_out == 0 && !saved.progress_wakes,
            "{}: {:?}",
            saved.id,
            saved.progress
        );
    }
    let released = request(room, "ada~bob", handoff);
    assert!(released.released);
    if let Some(thread) = &released.thread {
        assert_eq!(says_ended(room, thread), [true; 3], "{thread}");
    }
}

/// Whether a handed-over thread's link says its delegation has ended: on the
/// thread itself, on Bob's tape, where his conversation counts the handoffs
/// he is working on, and on Ada's copy.
pub(super) fn says_ended(room: &Room, thread: &str) -> [bool; 3] {
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

/// A handed-over thread's link says its delegation ended when, and only
/// when, it ends: not while it stands, a finished handoff the person goes
/// on with included, and no more once a new handoff into it stands.
#[tokio::test]
async fn a_threads_link_says_its_delegation_ended_when_and_only_when_it_does() {
    let (room, _agents, handoff, thread) = handed_over_and_done("progress-link-ended").await;
    assert_eq!(says_ended(&room, &thread), [false; 3], "done, it stands");
    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "and the date?", None)
        .await
        .unwrap();
    until("the person's turn is over", || {
        said_in(&room, &thread).len() == 2 && idle(&room, &thread)
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

    room.resume_exchange("ada", "bob").unwrap();
    let again = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"work again","intent":"handoff","thread":handoff}),
        )
        .await
        .unwrap();
    let again = serde_json::from_str::<serde_json::Value>(&again).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    done(&room, &again).await;
    assert_eq!(work_thread(&room, &again), thread);
    assert_eq!(
        says_ended(&room, &thread),
        [false; 3],
        "a new handoff stands"
    );
}

#[tokio::test]
async fn stop_exchange_and_revocation_end_the_reports_and_drop_the_unheard_ones() {
    let (room, handoff, thread) = with_unheard_reports("progress-stop-exchange").await;
    room.stop_exchange("ada", "bob").unwrap();
    is_released(&room, &handoff);
    talks_without_a_report(&room, &handoff, &thread).await;

    let (room, handoff, thread) = with_unheard_reports("progress-teammate-stopped").await;
    room.stop("bob").unwrap();
    is_released(&room, &handoff);
    talks_without_a_report(&room, &handoff, &thread).await;

    let (room, handoff, thread) = with_unheard_reports("progress-policy-change").await;
    room.reattach("ada").await.unwrap();
    is_released(&room, &handoff);
    talks_without_a_report(&room, &handoff, &thread).await;
}

/// Review Focus 4: the delegator is stopped, or removed in the order the
/// wire's delete does it, while the thread goes on.
#[tokio::test]
async fn a_stopped_or_removed_delegator_gets_no_report_and_nothing_breaks() {
    let (room, _agents, handoff, thread) = handed_over_and_done("progress-ada-stopped").await;
    room.stop("ada").unwrap();
    assert!(request(&room, "ada~bob", &handoff).released);
    talks_without_a_report(&room, &handoff, &thread).await;

    let (room, _agents, handoff, thread) = handed_over_and_done("progress-ada-removed").await;
    room.invalidate("ada").unwrap();
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "persona", "id": "ada", "deleted": true}),
        )
        .unwrap();
    room.forget("ada");
    assert!(request(&room, "ada~bob", &handoff).released);
    talks_without_a_report(&room, &handoff, &thread).await;
}

/// A lease that lapses under a waiting handoff stops it where the worker
/// finds it, and what its sender had not heard goes with it.
#[tokio::test]
async fn a_handoff_stopped_for_a_lapsed_lease_drops_its_unheard_reports() {
    let (room, handoff, _action) = suspended_handoff("progress-lapsed-lease").await;
    until("ada is told", || !notices(&room, "ada").is_empty()).await;
    assert_eq!(request(&room, "ada~bob", &handoff).progress.len(), 1);
    room.capability_epoch("bob").invalidate();
    room.wake_exchange("ada~bob");
    until("the handoff is stopped", || {
        request(&room, "ada~bob", &handoff).phase == Phase::Stopped
    })
    .await;
    is_released(&room, &handoff);
}

/// A link that ends takes the delegations over it with it: nothing is left
/// to cross, and nothing more is reported.
#[tokio::test]
async fn an_unlinked_desk_ends_the_delegations_with_its_teammates() {
    let (room, _agents) = setup("progress-unlinked");
    let mut handoff = saved_request("far", Intent::Handoff, Phase::Done);
    handoff.from = "grizzly-desk/lou".into();
    handoff.progress = vec!["Swatch 44 is ready.".into()];
    handoff.progress_left_out = 2;
    handoff.progress_wakes = true;
    let mut ask = saved_request("question", Intent::Ask, Phase::Done);
    ask.from = "grizzly-desk/lou".into();
    room.save_pair(&Pair {
        id: "bob~grizzly-desk/lou".into(),
        a: "bob".into(),
        b: "grizzly-desk/lou".into(),
        exchanges: 0,
        paused: false,
        requests: vec![handoff, ask],
    })
    .unwrap();
    let elsewhere = saved_request("near", Intent::Handoff, Phase::Done);
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: 0,
        paused: false,
        requests: vec![elsewhere],
    })
    .unwrap();

    room.stop_linked_exchanges("grizzly-desk", "grizzly");
    let far = room.exchange_pair("bob~grizzly-desk/lou").unwrap().requests;
    assert!(far[0].released);
    assert!(far[0].progress.is_empty() && far[0].progress_left_out == 0 && !far[0].progress_wakes);
    assert!(!far[1].released, "a question is no delegation");
    assert!(!request(&room, "ada~bob", "near").released);
}

#[test]
fn only_a_handoff_is_released_and_a_request_not_released_saves_without_it() {
    let mut handoff = saved_request("handoff", Intent::Handoff, Phase::Done);
    handoff.progress = vec!["Swatch 45 is ready.".into()];
    handoff.progress_left_out = 1;
    handoff.progress_wakes = true;
    handoff.thread = Some("t1".into());
    let mut ask = saved_request("ask", Intent::Ask, Phase::Done);
    ask.thread = Some("t2".into());
    let mut ended = Vec::new();
    assert!(release(&mut handoff, &mut ended));
    assert!(!release(&mut handoff, &mut ended), "it is released once");
    assert!(!release(&mut ask, &mut ended));
    assert!(handoff.released && !ask.released);
    assert_eq!(ended, ["t1"], "its thread's link is to say so");
    assert!(
        handoff.progress.is_empty() && handoff.progress_left_out == 0 && !handoff.progress_wakes
    );

    let saved = serde_json::to_value(&handoff).unwrap();
    assert_eq!(saved["released"], true);
    assert!(
        serde_json::to_value(&ask)
            .unwrap()
            .get("released")
            .is_none()
    );
    let mut old = serde_json::to_value(&ask).unwrap();
    old.as_object_mut().unwrap().remove("released");
    assert!(!serde_json::from_value::<Request>(old).unwrap().released);
}

#[test]
fn reports_sent_over_a_link_leave_the_ones_made_while_they_crossed() {
    let mut sending = saved_request("handoff", Intent::Handoff, Phase::Done);
    sending.progress = vec!["Swatch 46 is ready.".into(), "Swatch 47 is ready.".into()];
    sending.progress_left_out = 1;
    sending.progress_wakes = true;

    let mut kept = sending.clone();
    take_sent(&mut kept, &sending);
    assert!(kept.progress.is_empty() && kept.progress_left_out == 0 && !kept.progress_wakes);

    let mut kept = sending.clone();
    kept.progress.push("Swatch 48 is ready.".into());
    take_sent(&mut kept, &sending);
    assert_eq!(kept.progress, ["Swatch 48 is ready."]);
    assert_eq!(kept.progress_left_out, 0);

    // The limit dropped both that were sent and one made since.
    let mut kept = sending.clone();
    kept.progress = vec!["Swatch 49 is ready.".into()];
    kept.progress_left_out = 4;
    take_sent(&mut kept, &sending);
    assert_eq!(kept.progress, ["Swatch 49 is ready."]);
    assert_eq!(
        kept.progress_left_out, 1,
        "only the one never sent is left out"
    );
}
