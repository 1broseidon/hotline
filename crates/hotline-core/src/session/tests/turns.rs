//! A turn on the tape: what the person said, what the agent answered, and
//! the room's bookkeeping around it.

use super::*;

#[tokio::test]
async fn a_live_session_takes_a_changed_model_list_without_restarting() {
    let (driver, offered) = Scripted::new(spoken_turn()).with_room_models();
    let room = room("models-follow", Fake::new(driver));
    let started = room.start("ada").await.unwrap();
    let mut infos = room.subscribe_info();

    lock(&offered).push(ConfigChoice {
        id: "xai/grok-4.7".to_string(),
        name: "Grok 4.7".to_string(),
        description: None,
        group: None,
        account: None,
    });
    // A setting that has nothing to do with models is not a reason to look.
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "chapterIdleHours", "value": 4}),
        )
        .unwrap();
    crate::room::models_changed(&room.log);

    let info = tokio::time::timeout(Duration::from_secs(5), infos.recv())
        .await
        .expect("the picker was not refreshed")
        .unwrap();
    let ids: Vec<&str> = info.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["anthropic/claude", "xai/grok-4.7"]);
    assert_eq!(info.state, SessionState::Ready, "the session kept running");
    assert_eq!(info.session_id, started.session_id);
    assert!(
        infos.try_recv().is_err(),
        "the unrelated setting did not refresh the picker a second time"
    );
}

#[tokio::test]
async fn the_users_line_is_on_the_tape_first_and_every_update_lands_behind_it() {
    let room = room("a-turn", Fake::new(Scripted::new(spoken_turn())));
    let mut infos = room.subscribe_info();
    let mut deltas = room.subscribe_deltas();
    room.start("ada").await.unwrap();

    room.prompt("ada", "what is here?", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 5).await;

    assert_eq!(
        kinds(&events),
        ["user", "thought", "tool", "agent", "turn"],
        "the tool call and its result are one event, superseded by id"
    );
    assert_eq!(events[0]["text"], "what is here?");
    assert_eq!(events[1]["text"], "let me look");
    assert_eq!(events[2]["toolCallId"], "c1");
    assert_eq!(events[2]["status"], "completed");
    assert_eq!(
        events[2]["output"],
        json!([{"type": "text", "text": "AGENTS.md"}])
    );
    assert_eq!(events[3]["text"], "one file");
    assert_eq!(events[4]["stopReason"], "end_turn");

    // The state the roster shows: ready when it started, thinking while the
    // turn ran, ready again when it was over.
    let mut states = Vec::new();
    for _ in 0..3 {
        states.push(infos.recv().await.unwrap().state);
    }
    assert_eq!(
        states,
        [
            SessionState::Ready,
            SessionState::Thinking,
            SessionState::Ready
        ]
    );

    // The deltas went out and were never written down. The answer came
    // after a tool call, so it streamed as thinking and landed whole: a
    // message after the work may turn out to be narration, and the window
    // must never type a bubble that then vanishes.
    let mut streamed = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        streamed.push(delta);
    }
    assert_eq!(
        streamed,
        [
            StreamDelta::ThreadDelta {
                thread: ThreadId::dm("ada"),
                message_id: "m1".to_string(),
                kind: DeltaKind::Thought,
                text: "let me look".to_string(),
            },
            StreamDelta::ThreadDelta {
                thread: ThreadId::dm("ada"),
                message_id: "m2".to_string(),
                kind: DeltaKind::Thought,
                text: "one file".to_string(),
            },
        ]
    );
}

/// The person's line is `sent` the moment it is written and `read` once the
/// agent has produced anything with it in context — not when the driver took
/// it, because a prompt the model never got to would then wear a read tick.
#[tokio::test]
async fn a_line_is_sent_when_written_and_read_once_the_agent_answers_to_it() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    let room = room("receipts", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    room.prompt("ada", "second", None, None).await.unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "user"]);
    assert_eq!(events[0]["receipt"], "sent", "taken, not yet proven read");
    assert_eq!(
        events[1]["receipt"], "sent",
        "still waiting behind the turn"
    );

    gate.add_permits(1);
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "user", "turn"]);
    assert_eq!(events[0]["receipt"], "read", "the turn is proof");
    assert_eq!(events[1]["receipt"], "sent", "its own turn has not begun");

    gate.add_permits(1);
    let events = settled(&room, "ada", 4).await;
    assert_eq!(events[1]["receipt"], "read");
    assert_eq!(events[0]["receipt"], "read", "nothing un-reads");
}

/// A cancelled line was taken but never answered; it stays sent.
#[tokio::test]
async fn a_cancelled_turn_leaves_its_line_sent() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    driver.on_cancel = vec![Update::Turn {
        stop_reason: "cancelled".to_string(),
        usage: None,
    }];
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("receipts-cancel", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();
    // The driver has the line and is mid-turn when the person stops it.
    for _ in 0..200 {
        if !lock(&prompts).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(*lock(&prompts), ["first"]);
    room.cancel("ada").unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "turn"]);
    assert_eq!(events[0]["receipt"], "sent");
}

/// The agent's reaction lands on the person's last message and nowhere else.
#[tokio::test]
async fn a_reaction_lands_on_the_last_thing_the_person_said() {
    let room = room("react", Fake::new(Scripted::new(spoken_turn())));
    room.start("ada").await.unwrap();
    room.prompt("ada", "ship it", None, None).await.unwrap();
    settled(&room, "ada", 5).await;
    let tools = TeammateTools::new(&room, "ada");
    assert_eq!(
        tools
            .call("react", &json!({ "emoji": "👍" }))
            .await
            .unwrap(),
        "Reacted."
    );
    tools
        .call("react", &json!({ "emoji": "👍" }))
        .await
        .unwrap();
    tools
        .call("react", &json!({ "emoji": "🙏" }))
        .await
        .unwrap();
    assert!(
        tools
            .call("react", &json!({ "emoji": "ok" }))
            .await
            .is_err()
    );
    let events = settled(&room, "ada", 5).await;
    assert_eq!(kinds(&events), ["user", "thought", "tool", "agent", "turn"]);
    assert_eq!(events[0]["reactions"], json!(["👍", "🙏"]));
    assert_eq!(events[0]["receipt"], "read", "a reaction keeps the receipt");
}

#[tokio::test]
async fn a_computer_tool_image_lands_a_frame_on_the_tape() {
    let png = "AAAA";
    let image = crate::driver::ToolImage {
        data: png.into(),
        mime_type: "image/png".into(),
    };
    let room = room(
        "computer-frame",
        Fake::new(Scripted::new(vec![
            Update::ToolCall {
                call_id: "c1".to_string(),
                title: "computer__capture".to_string(),
                kind: "computer__capture".to_string(),
            },
            Update::ToolResult {
                call_id: "c1".to_string(),
                ok: true,
                output: format!("the tree\n{}", image.placeholder()),
                images: vec![image],
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ])),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "look", None, None).await.unwrap();
    let events = settled(&room, "ada", 4).await;
    assert_eq!(
        kinds(&events),
        ["user", "tool", "computer_frame", "turn"],
        "the frame lands immediately after the completed tool"
    );
    assert_eq!(events[1]["status"], "completed");
    let data_url = events[2]["dataUrl"]
        .as_str()
        .expect("a frame carries a data URL");
    assert!(data_url.starts_with("data:image/png;base64,"), "{data_url}");
    assert!(data_url.ends_with(png), "{data_url}");
}

#[tokio::test]
async fn another_servers_image_is_a_placeholder_not_a_frame() {
    let image = crate::driver::ToolImage {
        data: "AAAA".into(),
        mime_type: "image/png".into(),
    };
    let room = room(
        "other-frame",
        Fake::new(Scripted::new(vec![
            Update::ToolCall {
                call_id: "c1".to_string(),
                title: "echo__shout".to_string(),
                kind: "echo__shout".to_string(),
            },
            Update::ToolResult {
                call_id: "c1".to_string(),
                ok: true,
                output: image.placeholder(),
                images: vec![image],
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ])),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "look", None, None).await.unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "tool", "turn"]);
    assert!(
        events[1]["output"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[image image/png"),
        "{:?}",
        events[1]["output"]
    );
}

#[tokio::test]
async fn a_cancelled_turn_fails_the_tool_it_caught_in_flight() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![
        Update::ToolCall {
            call_id: "c1".to_string(),
            title: "shell sleep 600".to_string(),
            kind: "shell".to_string(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]);
    driver.gate = Some(gate.clone());
    driver.on_cancel = vec![Update::Turn {
        stop_reason: "aborted".to_string(),
        usage: None,
    }];
    let room = room("cancel", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "run the long thing", None, None)
        .await
        .unwrap();
    gate.add_permits(1);
    let events = settled(&room, "ada", 2).await;
    assert_eq!(events[1]["status"], "in_progress");

    room.cancel("ada").unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "tool", "turn"]);
    assert_eq!(
        events[1]["status"], "failed",
        "a tool the turn never heard back about did not complete"
    );
    assert_eq!(events[2]["stopReason"], "aborted");
}

#[tokio::test]
async fn a_prompt_during_a_turn_waits_for_the_turn_it_would_have_interrupted() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let reaches = agents.driver.reaches.clone();
    let room = room("queue", agents);
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    room.prompt("ada", "second", None, None).await.unwrap();

    // Both lines are on the tape at once: what was said is a fact as soon as
    // it was said, whatever the agent is busy with.
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "user"]);

    // One permit per turn: the second turn cannot have started until the
    // first one's ended, because the driver only ever ran one script at once.
    gate.add_permits(1);
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "user", "turn"]);
    gate.add_permits(1);
    let events = settled(&room, "ada", 4).await;
    assert_eq!(kinds(&events), ["user", "user", "turn", "turn"]);
    assert_eq!(*lock(&prompts), ["first", "second"]);
    // Reach is read from the roster at every prompt, not from the persona the
    // session was started with.
    assert_eq!(*lock(&reaches), [Reach::Machine, Reach::Machine]);
}

/// Every line the room hands the driver is a line the driver hears.
///
/// The claim on the driver and the queue behind it are one fact. Held apart,
/// there was a moment — between the driver finding nothing waiting and letting
/// go of the turn — in which a line was filed behind a turn that was already
/// over. The teammate then sat on it: it took the next thing said to shake it
/// loose, and if nothing else was said, nothing ever did.
///
/// That is why nothing is said after the hunt below: another line would heal
/// exactly the fault being looked for. The moment is nanoseconds wide, so this
/// hunts rather than hopes — each line is said the instant the driver takes
/// the one before it, and the spin after that walks the saying across the tail
/// of the turn. The hunting lines are nudges, because a nudge is the cheapest
/// thing the room can dispatch and a hunt needs thousands of shots; the tape's
/// own line is counted with them.
#[tokio::test(flavor = "multi_thread")]
async fn every_line_the_room_hands_the_driver_is_a_line_the_driver_heard() {
    /// How many shots the hunt takes, and how far past the turn's end the
    /// saying of each one is walked. A spin is tens of nanoseconds and a turn
    /// here is tens of microseconds, so the walk covers the whole of it.
    const SHOTS: usize = 60_000;
    const WALK: usize = 2_000;
    /// How long a line may wait for an idle driver before it is plainly stuck.
    const STUCK: Duration = Duration::from_secs(2);
    const TICK: &str = "tick";

    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.prompts.clone();
    let room = room("tight-loop", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();

    // The first prompt must not satisfy the first nudge's counter change.
    // Otherwise the hunt can finish one tick ahead of the driver it is measuring.
    tokio::time::timeout(Duration::from_secs(15), async {
        while lock(&prompts).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the initial prompt should reach the driver before the hunt");

    let hunting = {
        let room = room.clone();
        let prompts = prompts.clone();
        tokio::task::spawn_blocking(move || {
            let mut said = 0;
            for shot in 0..SHOTS {
                let taken = lock(&prompts).len();
                room.nudge("ada", TICK).unwrap();
                said += 1;
                let waited = std::time::Instant::now();
                while lock(&prompts).len() == taken {
                    if waited.elapsed() > STUCK {
                        return said;
                    }
                    std::hint::spin_loop();
                }
                for _ in 0..(shot % WALK) {
                    std::hint::spin_loop();
                }
            }
            said
        })
    };
    let said = hunting.await.unwrap();

    let heard = lock(&prompts).clone();
    let on_the_tape: Vec<String> = tape(&room, "ada")
        .iter()
        .filter(|event| event["kind"] == "user")
        .map(|event| event["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(on_the_tape, ["first"]);
    assert_eq!(
        heard
            .iter()
            .filter(|line| *line != TICK)
            .collect::<Vec<_>>(),
        on_the_tape.iter().collect::<Vec<_>>(),
        "a line on the tape never reached the driver"
    );
    assert_eq!(
        heard.iter().filter(|line| *line == TICK).count(),
        said,
        "the room handed the driver {said} lines and it heard fewer: one of \
         them is waiting on a turn that ended without it"
    );
}

/// Two callers bring a teammate up at the same moment — the wire and a
/// schedule firing are exactly these two — and between them there is one
/// session in one chapter.
///
/// Unserialised, both read a tape with no chapter open and both write a marker
/// onto it, and the teammate is left holding a chapter that will never close.
/// The window is the width of one file read, so a handful of teammates raced
/// in turn is enough to land in it.
#[tokio::test(flavor = "multi_thread")]
async fn two_starts_at_once_leave_one_session_in_one_chapter() {
    const TEAMMATES: usize = 6;
    let log = scratch("start-race");
    for teammate in 0..TEAMMATES {
        enrol(&log, &persona(&format!("ada{teammate}")));
    }
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());

    for teammate in 0..TEAMMATES {
        let id = format!("ada{teammate}");
        let together = Arc::new(std::sync::Barrier::new(2));
        let mut starting = Vec::new();
        for _ in 0..2 {
            let room = room.clone();
            let together = together.clone();
            let id = id.clone();
            starting.push(tokio::spawn(async move {
                together.wait();
                room.start(&id).await
            }));
        }
        for start in starting {
            start.await.unwrap().unwrap();
        }
        assert_eq!(
            markers(&room, &id).len(),
            1,
            "one start opened {id}'s chapter and the other joined it"
        );
        assert_eq!(room.info(&id).state, SessionState::Ready);
    }
    assert_eq!(
        lock(&agents.preambles).len(),
        TEAMMATES,
        "a second start on a teammate that was up built a second agent"
    );
}

/// A deleted teammate is forgotten whole: the agent is stopped and the
/// start gate is gone, so nothing is kept for an id that names nobody.
#[tokio::test]
async fn forgetting_a_teammate_stops_it_and_drops_its_gate() {
    let room = room("forget", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();
    assert!(lock(&room.starts).contains_key("ada"));

    room.forget("ada");

    assert!(
        lock(&room.sessions).get("ada").is_none(),
        "the agent kept running"
    );
    assert!(!lock(&room.starts).contains_key("ada"), "the gate was kept");
}

/// Two messages arriving on a closed chapter open one chapter between them,
/// and both are spoken to the session that chapter belongs to.
///
/// The gate is stop-then-start. Performed twice over, the second stop cancels
/// the session the first had just brought up, and the line behind it is said
/// to an agent that is no longer there. A handful of teammates raced in turn,
/// because the window is the width of one file read.
#[tokio::test(flavor = "multi_thread")]
async fn two_prompts_on_a_closed_chapter_open_one_and_both_are_heard() {
    const TEAMMATES: usize = 5;
    let log = scratch("chapter-gate-race");
    for teammate in 0..TEAMMATES {
        enrol(&log, &persona(&format!("ada{teammate}")));
    }
    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.prompts.clone();
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);

    for teammate in 0..TEAMMATES {
        let id = format!("ada{teammate}");
        room.start(&id).await.unwrap();
        room.start_fresh_chapter(&id, ChapterClose::User)
            .await
            .unwrap();
        lock(&prompts).clear();

        let together = Arc::new(std::sync::Barrier::new(2));
        let mut speaking = Vec::new();
        for line in ["one", "two"] {
            let room = room.clone();
            let together = together.clone();
            let id = id.clone();
            speaking.push(tokio::spawn(async move {
                together.wait();
                room.prompt(&id, line, None, None).await
            }));
        }
        for speaker in speaking {
            speaker.await.unwrap().unwrap();
        }

        let markers = markers(&room, &id);
        assert_eq!(
            markers.len(),
            2,
            "{id}'s closed chapter and the one both lines landed in"
        );
        assert!(markers[1].get("endedAt").is_none());

        for _ in 0..200 {
            if lock(&prompts).len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut heard = lock(&prompts).clone();
        heard.sort();
        assert_eq!(heard, ["one", "two"], "{id} heard both lines");
    }
}

#[tokio::test]
async fn a_teammate_with_no_session_is_idle_and_a_started_one_reports_its_driver() {
    let room = room("info", Fake::new(Scripted::new(Vec::new())));
    assert_eq!(room.info("ada"), idle_info("ada"));
    assert_eq!(room.info("ada").state, SessionState::Idle);

    let info = room.start("ada").await.unwrap();
    assert_eq!(info.agent_name.as_deref(), Some("Scripted"));
    assert_eq!(info.current_model_id.as_deref(), Some("anthropic/claude"));
    assert_eq!(room.info("ada"), info);

    let switched = room.set_model("ada", "anthropic/other").await.unwrap();
    assert_eq!(
        switched.current_model_id.as_deref(),
        Some("anthropic/other")
    );
    assert_eq!(room.info("ada"), switched);

    room.stop("ada").unwrap();
    assert_eq!(room.info("ada").state, SessionState::Idle);
    assert!(
        room.prompt("ada", "anyone there?", None, None)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn driver_metadata_notifications_refresh_roster_without_tape_events() {
    let (driver, changes) = Scripted::new(Vec::new()).with_info_changes();
    let room = room("driver-info", Fake::new(driver));
    let mut infos = room.subscribe_info();
    room.start("ada").await.unwrap();
    assert_eq!(infos.recv().await.unwrap().state, SessionState::Ready);

    let updated = DriverInfo {
        models: vec![ConfigChoice {
            id: "anthropic/sonnet".to_string(),
            name: "Sonnet".to_string(),
            description: None,
            group: None,
            account: None,
        }],
        current_model_id: "anthropic/sonnet".to_string(),
        model_label: Some("Model".to_string()),
        modes: vec![ConfigChoice {
            id: "build".to_string(),
            name: "Build".to_string(),
            description: None,
            group: None,
            account: None,
        }],
        current_mode_id: Some("build".to_string()),
        mode_label: Some("Runtime mode".to_string()),
        configs: vec![crate::contract::SessionConfig {
            id: "reasoning".to_string(),
            name: "Reasoning".to_string(),
            category: Some(crate::contract::SessionConfigCategory::Effort),
            current_id: Some("high".to_string()),
            options: vec![ConfigChoice {
                id: "high".to_string(),
                name: "High".to_string(),
                description: None,
                group: None,
                account: None,
            }],
        }],
        ..DriverInfo::default()
    };
    changes.send_replace(updated.clone());

    let changed = tokio::time::timeout(Duration::from_secs(2), infos.recv())
        .await
        .expect("the driver metadata update arrived")
        .expect("the info stream stayed open");
    assert_eq!(changed.state, SessionState::Ready);
    assert_eq!(changed.models, updated.models);
    assert_eq!(
        changed.current_model_id,
        Some("anthropic/sonnet".to_string())
    );
    assert_eq!(changed.modes, updated.modes);
    assert_eq!(changed.current_mode_id, Some("build".to_string()));
    assert_eq!(changed.configs, updated.configs);
    assert_eq!(room.info("ada"), changed);
    assert!(
        tape(&room, "ada")
            .into_iter()
            .all(|event| event["kind"] == "chapter")
    );

    // Once the session is stopped, a late notification from its driver cannot
    // update the idle roster or resurrect a picker.
    room.stop("ada").unwrap();
    assert_eq!(room.info("ada").state, SessionState::Idle);
    changes.send_replace(DriverInfo {
        current_model_id: "stale-model".to_string(),
        ..updated
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(room.info("ada").state, SessionState::Idle);
    assert_eq!(room.info("ada").current_model_id, None);
}

/// A teammate's directory is made when it starts, wherever it was pointed.
#[tokio::test(flavor = "multi_thread")]
async fn starting_a_teammate_makes_its_working_directory() {
    let log = scratch("makes-cwd");
    let mut ada = persona("ada");
    ada.cwd = log
        .root()
        .join("workspaces")
        .join("ada")
        .to_string_lossy()
        .into_owned();
    enrol(&log, &ada);
    let room = Room::new(log, Arc::new(DeskKeys));
    assert!(!std::path::Path::new(&ada.cwd).exists());
    room.start("ada").await.unwrap();
    assert!(std::path::Path::new(&ada.cwd).is_dir());
}

/// Docker not running is no reason for a teammate to go silent: it starts
/// without its computer, is not told it has one, and the tape says why and
/// how to bring it back. The grant stays on, so the next start tries again.
#[tokio::test]
async fn a_computer_that_cannot_start_leaves_the_teammate_answering_without_one() {
    let log = scratch("computer-down");
    let mut ada = persona("ada");
    ada.cwd = log
        .root()
        .join("workspaces")
        .join("ada")
        .to_string_lossy()
        .into_owned();
    ada.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: None,
        memory: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    enrol(&log, &ada);
    let agents = Fake::new(Scripted::new(spoken_turn()));
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );

    let info = room
        .start("ada")
        .await
        .expect("the teammate starts without its computer");
    assert_eq!(info.state, SessionState::Ready);
    assert!(
        room.session("ada").is_ok(),
        "a message now reaches a running session"
    );

    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(!preamble.contains("You have a computer"), "{preamble}");
    assert!(
        notices(&room, "ada").is_empty(),
        "the conversation carries on without a word about the computer"
    );
    assert!(
        room.persona("ada")
            .unwrap()
            .computer
            .is_some_and(|computer| computer.enabled),
        "the grant is kept, so the next start tries the computer again"
    );
}

#[tokio::test]
async fn aborted_and_revoked_handoffs_return_failure_not_success() {
    for reason in ["aborted", "revoked"] {
        let log = scratch(&format!("handoff-{reason}"));
        enrol(&log, &persona("ada"));
        enrol(&log, &persona("bob"));
        let agents = Fake::new(Scripted::turns(vec![vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "partial".into(),
                text: "Only partly done".into(),
            },
            Update::Turn {
                stop_reason: reason.into(),
                usage: None,
            },
        ]]));
        let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
        room.allow_sender("bob", "ada").unwrap();
        let result = crate::mcp::server::TeammateTools::new(&room, "ada")
            .call(
                "message_teammate",
                &json!({
                    "to": "bob", "message": "Do the work", "intent": "handoff"
                }),
            )
            .await
            .unwrap();
        let receipt: Value = serde_json::from_str(&result).unwrap();
        let request_id = receipt["requestId"].as_str().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(event) = room.tape("ada").into_iter().find(|event| {
                    event["kind"] == "delivery" && event["cause"]["requestId"] == request_id
                }) {
                    break event;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result["cause"]["status"], "failed", "{reason}: {result}");
        assert_eq!(result["text"], "Only partly done");
    }
}

/// A desk stopped for a restart keeps what the person said behind a running
/// turn, stops the turn that would not finish and says so, and the next room
/// on the same data hands the kept line on once.
#[tokio::test]
async fn a_restart_keeps_the_waiting_line_and_says_which_turn_it_cut_off() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let room = room("restart", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;
    room.prompt("ada", "second", None, None).await.unwrap();
    settled(&room, "ada", 2).await;

    let stopped = room.stop_for_restart(Duration::from_millis(200)).await;
    assert_eq!(stopped.kept, 1);
    assert_eq!(stopped.interrupted, ["ada"]);
    // Nothing new is admitted once the room is closing.
    let refused = room.prompt("ada", "third", None, None).await.unwrap_err();
    assert!(refused.contains("restarting"), "{refused}");
    let notice = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "notice")
        .expect("the cut-off turn is said on the tape");
    assert!(notice["text"].as_str().unwrap().contains("restarted"));
    assert!(room.log.root().join(stopping::PENDING_FILE).exists());
    let root = room.log.root().to_path_buf();
    drop(stopped);
    drop(room);

    // The next desk on the same data hands "second" on, and only that.
    let agents = Fake::new(Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]));
    let driver = agents.driver.clone();
    let next = Room::with_agents_and_computers(
        Log::open(root.clone()),
        Arc::new(DeskKeys),
        agents,
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    for _ in 0..200 {
        if !lock(&driver.prompts).is_empty() && !root.join(stopping::PENDING_FILE).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(*lock(&driver.prompts), ["second"]);
    assert!(!root.join(stopping::PENDING_FILE).exists());
    until_state(&next, "ada", SessionState::Ready).await;
    // "first" was never run again, and "second" was not written twice.
    let users: Vec<String> = tape(&next, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "user")
        .map(|event| event["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(users, ["first", "second"]);
}

/// No tab opens on a screen nobody is looking at, and nothing but a web link
/// is handed to the system's opener.
#[tokio::test]
async fn a_link_opens_only_for_a_person_at_this_computer() {
    let room = room("open-link", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();
    crate::wire::commands::PROMPT_CLIENT
        .scope(
            crate::contract::Client::Phone,
            room.prompt("ada", "open ketch.run", None, None),
        )
        .await
        .unwrap();
    let away = room
        .open_link("ada", "https://ketch.run")
        .await
        .unwrap_err();
    assert!(away.contains("not at this computer"), "{away}");
    for refused in ["file:///etc/passwd", "javascript:alert(1)", "ketch.run", ""] {
        let refused = room.open_link("ada", refused).await.unwrap_err();
        assert!(refused.contains("http or https"), "{refused}");
    }
}
