//! Each turn of a thread a teammate handed over that is not its result
//! reaches the teammate that handed it over as a report: shown at once, kept
//! until heard, and heard once, one summary per thread.
use super::gates::{ask_carol, ask_from, handed_off, idle, request};
use super::*;
use crate::contract::SideEnd;
use crate::session::exchanges::progress::{REPORT_CHARS, TurnEnd, turn_report};
use crate::session::runner::Driven;

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
fn heard_with(agents: &Fake, line: &str) -> Option<String> {
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
    assert!(saved.progress_wakes, "a turn that stops to wait may wake");
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

    room.continue_side(&work).await.unwrap();
    room.prompt_side(&work, "still there?", None).await.unwrap();
    until("ada's work thread hears its line", || {
        heard_with(&agents, "still there?").is_some()
    })
    .await;
    assert!(!heard_with(&agents, "still there?").unwrap().contains(&late));
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
