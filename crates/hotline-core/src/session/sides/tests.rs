use super::*;
use crate::contract::{DeltaKind, SessionCheckpoint, StreamDelta};
use crate::driver::rig::Said;
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::tests::{Fake, Scripted, enrol, persona, scratch};
use crate::test_support::until;
use crate::thread::SIDE_IDLE_MS;
use std::time::Duration;
use tokio::sync::Semaphore;

fn say(id: &str, text: &str) -> Update {
    Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    }
}

#[test]
fn a_list_previews_the_teammates_last_words_else_what_was_asked() {
    let asked = serde_json::json!({"kind": "user", "text": "  fix the\nCI badge "});
    let said = serde_json::json!({"kind": "agent", "text": "Done, it was the cache."});
    assert_eq!(preview_line(&[]), None);
    assert_eq!(
        preview_line(std::slice::from_ref(&asked)).as_deref(),
        Some("fix the CI badge")
    );
    assert_eq!(
        preview_line(&[asked, said]).as_deref(),
        Some("Done, it was the cache.")
    );
}

fn turn() -> Update {
    Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }
}

fn room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    Room::with_agents_and_computers(
        log,
        Arc::new(crate::session::tests::DeskKeys),
        agents,
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    )
}

/// What a client is sent of a thread's stream: its link as the marker the
/// kind has always had. The stored line is checked where it is the point.
fn side_stream(room: &Room, side_id: &str) -> Vec<Value> {
    room.log
        .load(&StreamId::Side(side_id.to_string()))
        .into_iter()
        .map(Link::wire)
        .collect()
}

fn tape(room: &Room) -> Vec<Value> {
    room.log
        .load(&StreamId::Tape("ada".to_string()))
        .into_iter()
        .map(Link::wire)
        .collect()
}

fn kinds(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|event| event["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn settled(room: &Arc<Room>, side_id: &str) {
    until("the side thread finishes its turn", || {
        room.sides.get(side_id).is_none_or(|side| !side.working())
    })
    .await;
}

/// Waits until the room's millisecond clock has moved on, so what happens
/// next has a later timestamp than what happened before.
async fn a_millisecond_passes() {
    let before = now_ms();
    until("the clock moves on", || now_ms() > before).await;
}

/// A handoff is read along: the person cannot type into it, live or
/// archived, and cannot bring it back; the desk says who to talk to.
#[tokio::test]
async fn a_handed_over_thread_refuses_the_persons_words_and_its_continuation() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "On it."), turn()]));
    let room = room("side-handed-over", agents.clone());
    let (live, launch) = room
        .bring_up(
            "ada",
            Start {
                side_id: new_id(),
                title: "Wire the gateway".to_string(),
                started: now_ms(),
                opener: Some(Opener {
                    persona_id: "mack".to_string(),
                    name: "Mack".to_string(),
                }),
                fresh: true,
                saved: None,
                handoff: None,
            },
        )
        .unwrap();
    room.launch(launch);
    let refused = room
        .prompt_side(&live.id, "do it my way", None)
        .await
        .unwrap_err();
    assert!(
        refused.starts_with("Mack handed this thread over"),
        "{refused}"
    );
    assert!(
        !kinds(&side_stream(&room, &live.id)).contains(&"user".to_string()),
        "a refused line is never written"
    );

    room.archive_side(&live.id, SideEnd::Person, None).unwrap();
    let refused = room.continue_side(&live.id).await.unwrap_err();
    assert!(refused.contains("Talk to Mack about it"), "{refused}");
    assert!(room.prompt_side(&live.id, "and now?", None).await.is_err());
}

#[tokio::test]
async fn a_side_thread_opened_empty_waits_and_is_named_by_the_first_line() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "On it."), turn()]));
    let room = room("side-empty", agents.clone());
    let summary = room.start_side("ada", "  ").await.unwrap();
    assert_eq!(summary.title, "");
    assert_eq!(summary.status, SideStatus::Live);
    // Its agent is still being brought up, which is a thread at work.
    assert!(summary.working);
    assert_eq!(kinds(&side_stream(&room, &summary.side_id)), ["side"]);
    settled(&room, &summary.side_id).await;
    assert!(!room.sides("ada")[0].working);

    room.prompt_side(&summary.side_id, "Fix the CI badge\nit is red", None)
        .await
        .unwrap();
    settled(&room, &summary.side_id).await;
    assert_eq!(room.sides("ada")[0].title, "Fix the CI badge");
    let stored = room.log.load(&StreamId::Tape("ada".to_string()));
    assert_eq!(stored.last().unwrap()["title"], "Fix the CI badge");

    // A later line does not rename it, and a reply keeps what it answers.
    room.prompt_side_replying(&summary.side_id, "Also the README", Some("m1".into()), None)
        .await
        .unwrap();
    assert_eq!(room.sides("ada")[0].title, "Fix the CI badge");
    let stream = side_stream(&room, &summary.side_id);
    let said = stream
        .iter()
        .rfind(|event| event["kind"] == "user")
        .unwrap();
    assert_eq!(said["text"], "Also the README");
    assert_eq!(said["replyTo"], "m1");
}

#[tokio::test]
async fn a_side_thread_runs_on_its_own_stream_and_leaves_one_marker_on_the_tape() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "On it."), turn()]));
    let room = room("side-start", agents.clone());
    let summary = room
        .start_side("ada", "Fix the CI badge\nit is red")
        .await
        .unwrap();
    assert_eq!(summary.title, "Fix the CI badge");
    assert_eq!(summary.status, SideStatus::Live);
    settled(&room, &summary.side_id).await;

    let stream = side_stream(&room, &summary.side_id);
    assert_eq!(kinds(&stream), ["side", "user", "agent", "turn"]);
    assert_eq!(stream[1]["text"], "Fix the CI badge\nit is red");
    assert_eq!(stream[2]["text"], "On it.");

    // The main tape holds the marker and none of the thread's words.
    let tape = tape(&room);
    assert_eq!(kinds(&tape), ["side"]);
    assert_eq!(tape[0]["id"], format!("link:side:{}", summary.side_id));
    assert_eq!(tape[0]["status"], "live");
    assert_eq!(tape[0]["personaId"], "ada");
    // Stored, it is a link; a client is sent the marker.
    let stored = room.log.load(&StreamId::Tape("ada".to_string()));
    assert_eq!(stored[0]["kind"], "link");
    assert_eq!(stored[0]["threadKind"], "side");
    assert_eq!(stored[0]["thread"], summary.side_id);
    assert_eq!(stored[0]["state"], "live");

    let sides = room.sides("ada");
    assert_eq!(sides.len(), 1);
    assert_eq!(sides[0].title, "Fix the CI badge");
    assert!(!sides[0].working);

    // Its own context: no seed, no checkpoint, no computer, and the brief.
    assert!(lock(&agents.seeds).last().unwrap().is_empty());
    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(preamble.contains("This is a work thread"), "{preamble}");
    assert!(preamble.contains("Another thread of"), "{preamble}");
    assert_eq!(agents.prompts()[0], "Fix the CI badge\nit is red");
}

#[tokio::test]
async fn the_brief_makes_a_thread_a_working_session_that_does_not_close_itself() {
    let agents = Fake::new(Scripted::new(vec![turn()]));
    let room = room("side-brief", agents.clone());
    let summary = room.start_side("ada", "Triage the repos").await.unwrap();
    settled(&room, &summary.side_id).await;
    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(!preamble.contains("When the task is done"), "{preamble}");
    assert!(preamble.contains("many requests"), "{preamble}");
    assert!(preamble.contains("do not suggest closing"), "{preamble}");
    assert!(preamble.contains("they said yes"), "{preamble}");
}

#[tokio::test]
async fn the_person_can_talk_in_a_side_thread_and_what_they_say_queues_behind_a_turn() {
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(
        Scripted::turns(vec![
            vec![say("m1", "First."), turn()],
            vec![say("m2", "Second."), turn()],
        ])
        .gated(gate.clone()),
    );
    let room = room("side-prompt", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    let id = summary.side_id;
    assert!(room.sides("ada")[0].working);
    room.prompt_side(&id, "And another thing", None)
        .await
        .unwrap();
    gate.add_permits(10);
    until("the second prompt is heard and answered", || {
        agents.prompts().len() == 2 && !room.sides("ada")[0].working
    })
    .await;
    let stream = side_stream(&room, &id);
    let said: Vec<&str> = stream
        .iter()
        .filter(|event| event["kind"] == "user" || event["kind"] == "agent")
        .map(|event| event["text"].as_str().unwrap())
        .collect();
    assert_eq!(said, ["Task", "And another thing", "First.", "Second."]);
    assert_eq!(agents.prompts().len(), 2);
    assert_eq!(agents.prompts()[1], "And another thing");
    assert_eq!(kinds(&tape(&room)), ["side"], "nothing of it on the tape");
}

#[tokio::test]
async fn live_words_are_addressed_by_side_id_and_never_by_teammate() {
    let agents = Fake::new(Scripted::new(vec![
        Update::Delta {
            kind: MessageKind::Agent,
            message_id: "m1".to_string(),
            text: "Hel".to_string(),
        },
        say("m1", "Hello."),
        turn(),
    ]));
    let room = room("side-deltas", agents);
    let mut deltas = room.subscribe_deltas();
    let summary = room.start_side("ada", "Task").await.unwrap();
    settled(&room, &summary.side_id).await;
    let mut seen = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        seen.push(delta);
    }
    assert_eq!(
        seen,
        [StreamDelta::ThreadDelta {
            thread: ThreadId::side(summary.side_id),
            message_id: "m1".to_string(),
            kind: DeltaKind::Text,
            text: "Hel".to_string(),
        }]
    );
}

/// A side thread's agent may write a call's tags in a typed reply: it
/// streams and is kept as its written version, without a tag.
#[tokio::test]
async fn a_reply_in_a_side_thread_is_shown_and_kept_as_its_written_version() {
    let delta = |text: &str| Update::Delta {
        kind: MessageKind::Agent,
        message_id: "m1".to_string(),
        text: text.to_string(),
    };
    let agents = Fake::new(Scripted::new(vec![
        delta("<spoken>Fixed.</spoken> <written"),
        delta(">Fixed. See the diff.</written>"),
        say(
            "m1",
            "<spoken>Fixed.</spoken> <written>Fixed. See the diff.</written>",
        ),
        turn(),
    ]));
    let room = room("side-tags", agents);
    let mut deltas = room.subscribe_deltas();
    let summary = room.start_side("ada", "Task").await.unwrap();
    settled(&room, &summary.side_id).await;
    let mut streamed = String::new();
    while let Ok(delta) = deltas.try_recv() {
        if let StreamDelta::ThreadDelta { text, .. } = delta {
            streamed.push_str(&text);
        }
    }
    assert_eq!(streamed, "Fixed. See the diff.");
    let said: Vec<String> = side_stream(&room, &summary.side_id)
        .iter()
        .filter(|event| event["kind"] == "agent")
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(said, ["Fixed. See the diff."]);
}

#[tokio::test]
async fn archiving_turns_the_marker_into_a_one_line_result_and_ends_the_agent() {
    let agents = Fake::new(Scripted::new(vec![
        say("m1", "The badge is green now."),
        turn(),
    ]));
    let room = room("side-archive", agents.clone());
    let summary = room.start_side("ada", "Fix the badge").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;
    room.archive_side(&id, SideEnd::Person, None).unwrap();
    assert!(room.sides("ada").is_empty(), "the chip goes");
    assert!(room.archive_side(&id, SideEnd::Person, None).is_ok());

    let tape = tape(&room);
    assert_eq!(tape.len(), 1);
    assert_eq!(tape[0]["status"], "archived");
    assert_eq!(tape[0]["archivedBy"], "person");
    assert_eq!(tape[0]["result"], "The badge is green now.");
    assert!(tape[0]["archivedAt"].as_i64().is_some());
    assert_eq!(tape[0]["ts"], summary.started_at, "it keeps its place");

    // Read-only from here on.
    assert!(room.prompt_side(&id, "more", None).await.is_err());
    let listed = room.side_threads("ada");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, SideStatus::Archived);
    assert_eq!(listed[0].result.as_deref(), Some("The badge is green now."));
    assert!(room.side_threads("someone-else").is_empty());
}

#[tokio::test]
async fn the_teammate_can_say_it_is_done_and_its_last_words_land_first() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "All done."), turn()]));
    let room = room("side-agent-archive", agents);
    let summary = room.start_side("ada", "Task").await.unwrap();
    let id = summary.side_id;
    let tools = TeammateTools::new(&room, "ada").for_work(id.clone());
    let said = tools
        .call(
            "archive_thread",
            &serde_json::json!({ "summary": "Badge fixed." }),
        )
        .await
        .unwrap();
    assert!(said.contains("Archiving"), "{said}");
    until("ada has no side threads", || room.sides("ada").is_empty()).await;
    assert!(room.sides("ada").is_empty());
    let stream = side_stream(&room, &id);
    assert!(stream.iter().any(|event| event["text"] == "All done."));
    assert_eq!(tape(&room)[0]["archivedBy"], "agent");
    assert_eq!(tape(&room)[0]["result"], "Badge fixed.");
}

#[tokio::test]
async fn a_side_threads_tools_are_its_own_set() {
    let agents = Fake::new(Scripted::new(vec![turn()]));
    let room = room("side-tools", agents);
    let summary = room.start_side("ada", "Task").await.unwrap();
    let tools = TeammateTools::new(&room, "ada").for_work(summary.side_id.clone());
    // Chapters belong to the main conversation; everything else a
    // teammate can do, a work thread can too.
    for refused in ["new_chapter", "resume_chapter"] {
        let error = tools
            .call(refused, &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(error.contains("chapters belong"), "{refused}: {error}");
    }
    let names: Vec<String> = tools
        .as_dynamic()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert!(names.contains(&"archive_thread".to_string()), "{names:?}");
    assert!(!names.contains(&"new_chapter".to_string()), "{names:?}");
    for full in ["schedule", "send_file", "message_teammate", "request_human"] {
        assert!(names.contains(&full.to_string()), "{full}: {names:?}");
    }
    // The main session's tools never offer it.
    let main = TeammateTools::new(&room, "ada");
    assert!(
        main.call("archive_thread", &serde_json::json!({ "summary": "x" }))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_limit_counts_running_agents_and_lets_go_of_the_idlest_for_a_new_one() {
    let agents = Fake::new(Scripted::new(vec![turn()]));
    let room = room("side-limit", agents);
    let first = room.start_side("ada", "One").await.unwrap();
    settled(&room, &first.side_id).await;
    a_millisecond_passes().await;
    let second = room.start_side("ada", "Two").await.unwrap();
    settled(&room, &second.side_id).await;
    a_millisecond_passes().await;
    let third = room.start_side("ada", "Three").await.unwrap();
    settled(&room, &third.side_id).await;
    assert_eq!(room.sides("ada").len(), MAX_LIVE);

    // One more parks the one that has waited longest; nothing is lost.
    a_millisecond_passes().await;
    let fourth = room.start_side("ada", "Four").await.unwrap();
    settled(&room, &fourth.side_id).await;
    let live: Vec<String> = room
        .sides("ada")
        .into_iter()
        .map(|side| side.title)
        .collect();
    assert_eq!(live, ["Two", "Three", "Four"]);
    let listed = room.side_threads("ada");
    let states: Vec<(&str, SideStatus)> = listed
        .iter()
        .map(|summary| (summary.title.as_str(), summary.status))
        .collect();
    assert_eq!(
        states,
        [
            ("Two", SideStatus::Live),
            ("Three", SideStatus::Live),
            ("Four", SideStatus::Live),
            ("One", SideStatus::Parked)
        ]
    );
    assert_eq!(tape(&room)[0]["status"], "parked");

    // Parked and archived threads are not agents: archiving one live
    // thread leaves room, and the parked one still does not count.
    room.archive_side(&second.side_id, SideEnd::Person, None)
        .unwrap();
    room.start_side("ada", "Five").await.unwrap();
    assert_eq!(room.sides("ada").len(), MAX_LIVE);
    assert!(room.start_side("nobody", "x").await.is_err());
}

#[tokio::test]
async fn a_third_thread_is_refused_only_while_both_places_are_mid_turn() {
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::new(vec![say("m1", "Working."), turn()]).gated(gate.clone()));
    let room = room("side-busy", agents);
    room.start_side("ada", "One").await.unwrap();
    room.start_side("ada", "Two").await.unwrap();
    room.start_side("ada", "Three").await.unwrap();
    let refused = room.start_side("ada", "Four").await.unwrap_err();
    assert!(refused.contains("already has 3"), "{refused}");
    assert!(refused.contains("working"), "{refused}");
    assert_eq!(room.sides("ada").len(), 3, "none was touched");
    gate.add_permits(100);
}

#[tokio::test]
async fn stopping_the_teammate_archives_its_side_threads_as_stopped() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "Working."), turn()]));
    let room = room("side-stop", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    settled(&room, &summary.side_id).await;
    let lease = room.sides.get(&summary.side_id).unwrap().capability.clone();
    let tools = TeammateTools::new(&room, "ada")
        .for_work(summary.side_id.clone())
        .with_capability(lease);
    room.stop("ada").unwrap();
    assert!(room.sides("ada").is_empty());
    assert_eq!(tape(&room)[0]["archivedBy"], "stopped");
    assert!(
        room.prompt_side(&summary.side_id, "hi", None)
            .await
            .is_err()
    );
    assert!(
        tools
            .call("search_thread", &serde_json::json!({ "query": "anything" }))
            .await
            .is_err(),
        "the thread's tool handles are dead"
    );
    assert!(agents.cancel_count() >= 1, "its agent was stopped");
}

#[tokio::test]
async fn a_policy_change_and_a_removal_revoke_side_threads_too() {
    let agents = Fake::new(Scripted::new(vec![turn()]));
    let room = room("side-revoke", agents);
    room.start_side("ada", "One").await.unwrap();
    room.invalidate("ada").unwrap();
    assert!(room.sides("ada").is_empty());
    // Refused while the policy is being rewritten; fine again afterwards.
    assert!(room.start_side("ada", "Two").await.is_err());
    room.capability_epoch("ada").activate();
    room.start_side("ada", "Three").await.unwrap();
    room.invalidate_all().unwrap();
    assert!(room.sides("ada").is_empty());
    room.capability_epoch("ada").activate();
    room.start_side("ada", "Four").await.unwrap();
    room.forget("ada");
    assert!(room.sides("ada").is_empty());
}

#[tokio::test]
async fn a_chapter_changing_in_the_main_conversation_leaves_a_side_thread_alone() {
    let agents = Fake::new(Scripted::new(vec![turn()]));
    let room = room("side-chapter", agents);
    let summary = room.start_side("ada", "Task").await.unwrap();
    room.stop_session("ada");
    assert_eq!(room.sides("ada").len(), 1);
    room.prompt_side(&summary.side_id, "still here?", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_idle_side_thread_is_parked_after_a_few_hours_but_not_a_working_one() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "Done for now."), turn()]));
    let room = room("side-idle", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    settled(&room, &summary.side_id).await;
    room.sweep_threads(now_ms() + SIDE_IDLE_MS - 60_000);
    assert_eq!(room.sides("ada").len(), 1, "not yet");
    room.sweep_threads(now_ms() + SIDE_IDLE_MS + 60_000);
    assert!(room.sides("ada").is_empty(), "the agent is let go of");
    assert!(agents.cancel_count() >= 1);

    // Still open: parked, not archived, with nothing made of an ending.
    let tape = tape(&room);
    assert_eq!(tape.len(), 1);
    assert_eq!(tape[0]["status"], "parked");
    assert!(tape[0].get("archivedBy").is_none());
    assert!(tape[0].get("result").is_none());
    let listed = room.side_threads("ada");
    assert_eq!(listed[0].status, SideStatus::Parked);
    assert!(
        room.archive_side(&summary.side_id, SideEnd::Person, None)
            .is_ok()
    );
}

#[tokio::test]
async fn the_person_parks_a_live_thread_and_only_an_open_one() {
    let agents = Fake::new(Scripted::new(vec![say("m1", "Done for now."), turn()]));
    let room = room("side-park", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;

    room.park_side_thread(&id).unwrap();
    assert!(room.sides("ada").is_empty(), "the agent is let go of");
    assert_eq!(tape(&room)[0]["status"], "parked");
    assert!(tape(&room)[0].get("archivedBy").is_none());
    room.park_side_thread(&id)
        .expect("parking a parked thread is not an error");

    room.archive_side(&id, SideEnd::Person, None).unwrap();
    assert!(
        room.park_side_thread(&id).is_err(),
        "an archived one is not open"
    );
    assert!(room.park_side_thread("ghost").is_err());
}

#[tokio::test]
async fn a_line_said_to_a_parked_thread_brings_it_back_from_its_own_stream() {
    let agents = Fake::new(Scripted::turns(vec![
        vec![say("m1", "Repo one is clean."), turn()],
        vec![say("m2", "Repo two has a stale lockfile."), turn()],
    ]));
    let room = room("side-wake", agents.clone());
    let summary = room.start_side("ada", "Triage my repos").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;
    room.sweep_threads(now_ms() + SIDE_IDLE_MS + 60_000);
    assert!(room.sides("ada").is_empty());

    room.prompt_side(&id, "Now repo two", None).await.unwrap();
    assert_eq!(
        room.sides("ada").len(),
        1,
        "an agent again, without a button"
    );
    settled(&room, &id).await;
    assert_eq!(tape(&room)[0]["status"], "live");
    assert_eq!(agents.prompts().last().unwrap(), "Now repo two");

    // Hotline Agent has no session of its own to reopen: it was handed
    // what the thread had said as its history.
    let seeds = lock(&agents.seeds).clone();
    assert_eq!(seeds.len(), 2);
    assert!(seeds[0].is_empty());
    let heard: Vec<String> = crate::session::tests::words(seeds[1].clone())
        .into_iter()
        .map(|line| match line {
            Said::User(text) | Said::Agent(text) => text,
        })
        .collect();
    assert_eq!(heard, ["Triage my repos", "Repo one is clean."]);
    let stream = side_stream(&room, &id);
    let said: Vec<&str> = stream
        .iter()
        .filter(|event| event["kind"] == "user" || event["kind"] == "agent")
        .map(|event| event["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        said,
        [
            "Triage my repos",
            "Repo one is clean.",
            "Now repo two",
            "Repo two has a stale lockfile."
        ]
    );
    assert_eq!(kinds(&tape(&room)), ["side"], "nothing of it on the tape");
}

#[tokio::test]
async fn an_archived_thread_stays_read_only_until_it_is_continued() {
    let agents = Fake::new(Scripted::turns(vec![
        vec![say("m1", "Fixed."), turn()],
        vec![say("m2", "Also this."), turn()],
    ]));
    let room = room("side-continue", agents);
    let summary = room.start_side("ada", "Fix the badge").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;
    room.archive_side(&id, SideEnd::Person, None).unwrap();
    let err = room.prompt_side(&id, "more", None).await.unwrap_err();
    assert!(err.contains("Continue"), "{err}");

    let back = room.continue_side(&id).await.unwrap();
    assert_eq!(back.status, SideStatus::Live);
    assert_eq!(back.title, "Fix the badge");
    assert_eq!(back.started_at, summary.started_at, "it keeps its place");
    assert_eq!(tape(&room).len(), 1);
    assert_eq!(tape(&room)[0]["status"], "live");
    assert!(tape(&room)[0].get("result").is_none(), "no stale result");
    room.prompt_side(&id, "And this", None).await.unwrap();
    settled(&room, &id).await;
    assert!(
        side_stream(&room, &id)
            .iter()
            .any(|event| event["text"] == "Also this.")
    );
    // Continuing a live one is answered as it is.
    assert_eq!(
        room.continue_side(&id).await.unwrap().status,
        SideStatus::Live
    );
    assert!(room.continue_side("missing").await.is_err());
}

#[tokio::test]
async fn a_child_thread_saves_its_session_after_a_turn_and_reopens_it() {
    let agents = Fake::new(Scripted::turns(vec![
        vec![say("m1", "On it."), turn()],
        vec![say("m2", "Yes."), turn()],
    ]));
    agents.reporting("s-side", true);
    let log = scratch("side-session");
    let mut ada = persona("ada");
    ada.backend_id = "cursor".to_string();
    ada.cwd = log.root().join("ada").to_string_lossy().into_owned();
    // The teammate's own session is never the thread's.
    ada.session_checkpoints = vec![SessionCheckpoint {
        backend_id: "cursor".to_string(),
        session_id: "main".to_string(),
    }];
    enrol(&log, &ada);
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(crate::session::tests::DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    let summary = room.start_side("ada", "Triage").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;
    assert!(agents.views()[0].session_checkpoints.is_empty());
    let marker = tape(&room)[0].clone();
    assert_eq!(marker["sessionId"], "s-side");
    assert_eq!(marker["backendId"], "cursor");
    assert_eq!(side_stream(&room, &id)[0]["sessionId"], "s-side");

    room.sweep_threads(now_ms() + SIDE_IDLE_MS + 60_000);
    room.prompt_side(&id, "Next", None).await.unwrap();
    settled(&room, &id).await;
    let views = agents.views();
    let reopened = &views[1].session_checkpoints;
    assert_eq!(reopened.len(), 1);
    assert_eq!(
        (
            reopened[0].backend_id.as_str(),
            reopened[0].session_id.as_str()
        ),
        ("cursor", "s-side")
    );
    assert_eq!(views.len(), 2, "recall worked, so nothing was replayed");
    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(!preamble.contains("hotline_side_transcript"), "{preamble}");
    assert!(lock(&agents.seeds).iter().all(Vec::is_empty));
}

#[tokio::test]
async fn a_child_that_cannot_reopen_a_thread_is_given_its_transcript_instead() {
    let agents = Fake::new(Scripted::turns(vec![
        vec![say("m1", "The lockfile is stale."), turn()],
        vec![say("m2", "Noted."), turn()],
    ]));
    agents.reporting("s-side", false);
    let log = scratch("side-transcript");
    let mut ada = persona("ada");
    ada.backend_id = "cursor".to_string();
    ada.cwd = log.root().join("ada").to_string_lossy().into_owned();
    enrol(&log, &ada);
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(crate::session::tests::DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    let summary = room.start_side("ada", "Triage").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;
    // Never claimed as recall: the harness said it did not restore.
    assert!(tape(&room)[0].get("sessionId").is_some());
    room.archive_side(&id, SideEnd::Person, None).unwrap();
    room.continue_side(&id).await.unwrap();
    settled(&room, &id).await;
    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(preamble.contains("hotline_side_transcript"), "{preamble}");
    assert!(preamble.contains("The lockfile is stale."), "{preamble}");
    assert!(preamble.contains("Triage"), "{preamble}");
    let views = agents.views();
    assert!(
        views.last().unwrap().session_checkpoints.is_empty(),
        "the retry starts clean"
    );
    // The new session is not promised until a turn has run on it.
    assert!(tape(&room)[0].get("sessionId").is_none());
}

#[tokio::test]
async fn a_closing_note_is_written_on_archive_and_found_by_search() {
    let agents = Fake::answering(
        Scripted::new(vec![say("m1", "Pinned them."), turn()]),
        crate::session::tests::note_json("Harden CI workflow"),
    );
    let room = room("side-note", agents);
    let summary = room.start_side("ada", "Look at CI").await.unwrap();
    let id = summary.side_id;
    settled(&room, &id).await;
    room.archive_side(&id, SideEnd::Person, None).unwrap();
    until("the tape's marker has its note", || {
        tape(&room)[0].get("note").is_some()
    })
    .await;
    let marker = tape(&room)[0].clone();
    let note = marker["note"].as_str().unwrap();
    assert!(note.contains("Goal: Get the crane moving"), "{note}");
    assert!(note.contains("Outcome: It moved."), "{note}");
    assert!(note.contains("oil the winch"), "{note}");
    assert!(note.contains("Files: crane.log"), "{note}");
    // The main tape line is the title and one line of how it came out.
    assert_eq!(marker["title"], "Harden CI workflow");
    assert_eq!(marker["result"], "It moved.");
    assert_eq!(marker["status"], "archived");
    assert_eq!(side_stream(&room, &id)[0]["note"], marker["note"]);
    assert_eq!(listed_title(&room), "Harden CI workflow");

    let hits = crate::store::search::search(room.log.root(), "ada", "winch", Some(5)).unwrap();
    let text = serde_json::to_string(&hits).unwrap();
    assert!(text.contains(&format!("side:{id}")), "{text}");
}

fn listed_title(room: &Room) -> String {
    room.side_threads("ada")[0].title.clone()
}

#[tokio::test]
async fn what_the_teammate_wrote_as_the_result_survives_the_note() {
    let agents = Fake::answering(
        Scripted::new(vec![say("m1", "All done."), turn()]),
        crate::session::tests::note_json("Badge work"),
    );
    let room = room("side-note-result", agents);
    let summary = room.start_side("ada", "Task").await.unwrap();
    settled(&room, &summary.side_id).await;
    room.archive_side(
        &summary.side_id,
        SideEnd::Agent,
        Some("Badge fixed.".into()),
    )
    .unwrap();
    until("the tape's marker has its note", || {
        tape(&room)[0].get("note").is_some()
    })
    .await;
    assert_eq!(tape(&room)[0]["result"], "Badge fixed.");
    assert_eq!(tape(&room)[0]["title"], "Badge work");
}

#[tokio::test]
async fn an_archived_thread_is_found_by_search_as_the_line_it_left_behind() {
    let agents = Fake::new(Scripted::new(vec![
        say("m1", "Pinned the action versions."),
        turn(),
    ]));
    let room = room("side-search", agents);
    let summary = room.start_side("ada", "Harden the workflow").await.unwrap();
    settled(&room, &summary.side_id).await;
    room.archive_side(&summary.side_id, SideEnd::Person, None)
        .unwrap();
    let hits = crate::store::search::search(room.log.root(), "ada", "workflow", Some(5)).unwrap();
    let text = serde_json::to_string(&hits).unwrap();
    assert!(text.contains("Harden the workflow"), "{text}");
    assert!(
        text.contains(&format!("side:{}", summary.side_id)),
        "{text}"
    );
}

#[tokio::test]
async fn the_context_a_thread_opens_with_is_the_main_chapters_note_and_its_last_lines() {
    let agents = Fake::new(Scripted::new(vec![turn()]));
    let room = room("side-context", agents.clone());
    let tape_id = StreamId::Tape("ada".to_string());
    for event in [
        serde_json::json!({"kind":"chapter","id":"c1","ts":1,"backendId":"hotline","endedAt":2,
            "title":"Crane repair","note":"Goal: fix the crane. Outcome: winch oiled."}),
        serde_json::json!({"kind":"chapter","id":"c2","ts":3,"backendId":"hotline"}),
        serde_json::json!({"kind":"user","id":"u1","ts":4,"text":"How is the harbour?"}),
        serde_json::json!({"kind":"agent","id":"a1","ts":5,"text":"Quiet today."}),
    ] {
        room.log.append(&tape_id, &event).unwrap();
    }
    let summary = room
        .start_side("ada", "Check the tide tables")
        .await
        .unwrap();
    settled(&room, &summary.side_id).await;
    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(preamble.contains("winch oiled"), "{preamble}");
    assert!(preamble.contains("Quiet today."), "{preamble}");
    assert!(
        !preamble.contains("Check the tide tables"),
        "the task is the first message"
    );
}

#[tokio::test]
async fn a_permission_card_in_a_thread_is_answered_by_side_id() {
    let gate = Arc::new(Semaphore::new(1));
    let agents = Fake::new(
        Scripted::new(vec![
            Update::Permission {
                request_id: "r1".to_string(),
                title: "Run the tests?".to_string(),
                options: vec![PermissionOption {
                    option_id: "allow".to_string(),
                    name: "Allow".to_string(),
                    kind: None,
                }],
            },
            turn(),
        ])
        .gated(gate.clone()),
    );
    agents.awaiting("r1");
    let room = room("side-permission", agents);
    let summary = room.start_side("ada", "Task").await.unwrap();
    until("a permission card on the side's stream", || {
        side_stream(&room, &summary.side_id)
            .iter()
            .any(|event| event["kind"] == "permission")
    })
    .await;
    assert!(
        room.side_threads("ada")[0].waiting,
        "the list says it is stopped on a card"
    );
    room.answer_side_permission(&summary.side_id, "r1", "allow")
        .await
        .unwrap();
    let card = side_stream(&room, &summary.side_id)
        .into_iter()
        .find(|event| event["kind"] == "permission")
        .unwrap();
    assert_eq!(card["decision"], "allow");
    assert_eq!(card["decidedOptionName"], "Allow");
    assert!(
        tape(&room)
            .iter()
            .all(|event| event["kind"] != "permission"),
        "never on the main tape"
    );
    assert!(
        room.answer_side_permission(&summary.side_id, "r1", "allow")
            .await
            .is_err(),
        "answerable once"
    );
}

/// Waits until the room's agents have heard `count` lines.
async fn heard(agents: &Fake, count: usize) {
    until(&format!("the agent hears {count} lines"), || {
        agents.prompts().len() >= count
    })
    .await;
}

#[tokio::test]
async fn a_thread_opens_before_its_agent_is_up_and_the_first_line_waits_for_it() {
    let gate = Arc::new(Semaphore::new(0));
    let agents =
        Fake::new(Scripted::new(vec![say("m1", "On it."), turn()]).slow_to_start(gate.clone()));
    let room = room("side-slow-start", agents.clone());

    // The agent cannot start yet, and the thread is open all the same.
    let summary = tokio::time::timeout(
        Duration::from_secs(2),
        room.start_side("ada", "Fix the CI badge"),
    )
    .await
    .expect("opening a thread does not wait for its agent")
    .unwrap();
    assert_eq!(summary.status, SideStatus::Live);
    assert!(summary.working, "a thread starting is shown at work");
    let stream = side_stream(&room, &summary.side_id);
    assert_eq!(kinds(&stream), ["side", "user"]);
    assert_eq!(tape(&room)[0]["status"], "live");
    assert!(agents.prompts().is_empty());

    // A line said while it starts is written at once and waits its turn.
    room.prompt_side(&summary.side_id, "And the README", None)
        .await
        .unwrap();
    assert_eq!(
        kinds(&side_stream(&room, &summary.side_id)),
        ["side", "user", "user"]
    );
    assert!(agents.prompts().is_empty());

    gate.add_permits(1);
    heard(&agents, 2).await;
    settled(&room, &summary.side_id).await;
    assert_eq!(agents.prompts(), ["Fix the CI badge", "And the README"]);
    // Neither line was also handed over as history.
    assert!(lock(&agents.seeds).last().unwrap().is_empty());
}

#[tokio::test]
async fn an_agent_that_cannot_start_says_so_in_the_thread_and_the_thread_is_put_away() {
    let agents =
        Fake::new(Scripted::new(vec![turn()]).failing_to_start("the harness is not installed"));
    let room = room("side-failed-start", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    until("ada has no side threads", || room.sides("ada").is_empty()).await;
    assert!(room.sides("ada").is_empty(), "no agent is left behind");
    assert!(agents.prompts().is_empty());

    let stream = side_stream(&room, &summary.side_id);
    let notice = stream
        .iter()
        .find(|event| event["kind"] == "notice")
        .expect("the thread says why");
    assert_eq!(notice["level"], "error");
    assert!(
        notice["text"]
            .as_str()
            .unwrap()
            .contains("the harness is not installed"),
        "{notice}"
    );
    let stored = room.log.load(&StreamId::Tape("ada".to_string()));
    assert_eq!(stored.last().unwrap()["state"], "closed");
    assert_eq!(stored.last().unwrap()["end"], "failed");
    assert!(
        stored.last().unwrap()["outcome"]
            .as_str()
            .unwrap()
            .contains("the harness is not installed")
    );
    // Its place is free again.
    assert!(room.sides.has_room("ada"));
    assert!(!room.work_is_open(&summary.side_id));
}

#[tokio::test]
async fn a_thread_ended_while_its_agent_starts_stays_ended() {
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::new(vec![turn()]).slow_to_start(gate.clone()));
    let room = room("side-ended-starting", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    room.archive_side(&summary.side_id, SideEnd::Person, None)
        .unwrap();
    gate.add_permits(1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(room.sides("ada").is_empty());
    assert!(agents.prompts().is_empty(), "the task is not run");
    assert!(
        !side_stream(&room, &summary.side_id)
            .iter()
            .any(|event| event["kind"] == "notice"),
        "ending it on purpose is not a failed start"
    );
    assert!(!room.work_is_open(&summary.side_id));
}

#[tokio::test]
async fn cancelling_stops_the_turn_and_keeps_the_thread() {
    let gate = Arc::new(Semaphore::new(0));
    let script = Scripted::new(vec![say("m1", "Starting."), turn()])
        .gated(gate)
        .on_cancel(vec![Update::Turn {
            stop_reason: "aborted".to_string(),
            usage: None,
        }]);
    let agents = Fake::new(script);
    let room = room("side-cancel", agents.clone());
    let summary = room.start_side("ada", "Task").await.unwrap();
    until("the turn starts", || agents.prompts().len() == 1).await;
    room.cancel_side(&summary.side_id).unwrap();
    settled(&room, &summary.side_id).await;
    assert_eq!(room.sides("ada").len(), 1);
    assert!(agents.cancel_count() >= 1);
    assert!(
        side_stream(&room, &summary.side_id)
            .iter()
            .any(|event| event["stopReason"] == "aborted")
    );
}
