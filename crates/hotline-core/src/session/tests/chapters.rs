//! Chapters: where one working context ends and the next begins.

use super::*;

/// Nothing said is outside a chapter, and a chapter that is still open is the
/// chapter a restarted session rejoins.
#[tokio::test]
async fn a_session_opens_a_chapter_and_a_restart_within_it_opens_no_second_one() {
    let room = room("chapter-open", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();

    let opened = markers(&room, "ada");
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0]["backendId"], "hotline");
    assert_eq!(
        opened[0].get("endedAt"),
        None,
        "a fresh marker is the open one"
    );
    assert_eq!(
        opened[0].get("sessionId"),
        None,
        "Hotline Agent has no checkpoint"
    );

    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    assert_eq!(markers(&room, "ada"), opened);
}

/// An idle teammate has no driver to rebuild; the next start reads the new
/// state anyway.
#[tokio::test]
async fn reattach_on_an_idle_teammate_does_nothing() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("reattach-idle", agents.clone());
    room.reattach("ada").await.unwrap();
    assert!(
        lock(&agents.preambles).is_empty(),
        "an idle teammate built a driver"
    );
    assert_eq!(room.info("ada").state, SessionState::Idle);
}

/// Between turns the old driver is cancelled, a new one is built, and the
/// open chapter is the one the new session joins.
#[tokio::test]
async fn reattach_on_a_ready_session_swaps_the_driver_and_keeps_the_chapter() {
    let driver = Scripted::new(Vec::new());
    *lock(&driver.session_id) = Some("s-1".to_string());
    let agents = Fake::new(driver);
    let cancels = agents.driver.cancels.clone();
    let session_id = agents.driver.session_id.clone();
    let room = room("reattach-ready", agents.clone());
    room.start("ada").await.unwrap();
    let opened = markers(&room, "ada");
    assert_eq!(room.info("ada").session_id.as_deref(), Some("s-1"));
    assert_eq!(lock(&agents.preambles).len(), 1);

    // The next start reports a different id, as a new child would.
    *lock(&session_id) = Some("s-2".to_string());
    room.reattach("ada").await.unwrap();

    assert!(*lock(&cancels) > 0, "the old driver saw cancel");
    assert_eq!(lock(&agents.preambles).len(), 2, "a new driver was built");
    assert_eq!(room.info("ada").session_id.as_deref(), Some("s-2"));
    assert_eq!(markers(&room, "ada"), opened, "the chapter stayed open");
}

/// A failed policy write leaves its generation closed. Its retained record
/// must neither claim to be ready nor silently accept work under old grants.
#[tokio::test]
async fn an_unfinished_policy_update_refuses_work_until_reattached() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("unfinished-policy", agents.clone());
    room.start("ada").await.unwrap();
    room.invalidate("ada").unwrap();

    assert_eq!(room.info("ada").state, SessionState::Stopped);
    assert!(room.start("ada").await.is_err());
    assert!(
        room.prompt("ada", "must not run", None, None)
            .await
            .is_err()
    );
    assert_eq!(lock(&agents.preambles).len(), 1);

    room.reattach("ada").await.unwrap();
    assert_eq!(room.info("ada").state, SessionState::Ready);
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// Stop also covers a replacement between detaching its predecessor and
/// entering the driver's startup future. A later explicit start still works.
#[tokio::test]
async fn stop_revokes_a_replacement_before_its_startup_begins() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("stop-replacement", agents.clone());
    room.start("ada").await.unwrap();
    let replacement = room.stop_with_capability("ada");
    room.stop("ada").unwrap();

    assert!(room.start_now("ada", replacement).await.is_err());
    assert_eq!(lock(&agents.preambles).len(), 1);
    room.start("ada").await.unwrap();
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// A routine stop during the wire's invalidate-to-append window keeps the
/// epoch quarantined. A start cannot revive a session with the old policy;
/// reattach is the operation that activates the replacement generation.
#[tokio::test]
async fn stop_during_policy_quarantine_cannot_revive_the_old_generation() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("stop-quarantine", agents.clone());
    room.start("ada").await.unwrap();
    let old = lock(&room.sessions).get("ada").cloned().unwrap();

    room.invalidate("ada").unwrap();
    room.stop("ada").unwrap();
    assert!(!old.capability.is_current(), "the old lease was revived");
    assert!(
        room.start("ada").await.is_err(),
        "start reopened an epoch still waiting for the policy append"
    );

    room.reattach("ada").await.unwrap();
    room.start("ada").await.unwrap();
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// A policy change revokes the turn in flight and drops lines queued behind
/// it. The replacement session is built immediately, so a line already
/// handed to the old session cannot run with its former tools.
#[tokio::test]
async fn reattach_during_a_turn_cancels_the_old_queue_before_rebuilding() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
    ]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("reattach-defer", agents.clone());
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;

    room.prompt("ada", "second", None, None).await.unwrap();

    room.reattach("ada").await.unwrap();
    assert_eq!(
        lock(&agents.preambles).len(),
        2,
        "the replacement was built without waiting for the old turn"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        *lock(&prompts),
        ["first"],
        "the queued line never reached the replacement driver"
    );
}

/// The gate: a chapter that closed took the agent's context with it, so the
/// next message reaches a session that has never seen it.
#[tokio::test]
async fn a_prompt_after_an_asked_close_starts_a_fresh_session_in_a_new_chapter() {
    let agents = Fake::answering(
        Scripted::turns(vec![
            saying("one", "It jammed."),
            saying("two", "All clear."),
        ]),
        note_json("Crane jam"),
    );
    let room = room("chapter-rotate", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "did it jam?", None, None).await.unwrap();
    settled(&room, "ada", 3).await;

    let closed = room
        .start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert_eq!(closed.title.as_deref(), Some("Crane jam"));
    assert_eq!(closed.messages, 2);

    room.prompt("ada", "and now?", None, None).await.unwrap();
    settled(&room, "ada", 6).await;

    assert_eq!(
        kinds(&tape(&room, "ada")),
        [
            "chapter", "user", "agent", "turn", "chapter", "user", "agent", "turn"
        ],
        "the new marker opens before the line that lands in it"
    );
    let markers = markers(&room, "ada");
    assert_eq!(markers[0]["closedBy"], "user");
    assert_eq!(markers[1].get("endedAt"), None);

    // Two agents were built, and the second was seeded with nothing: the
    // chapter it joined has no conversation in it yet.
    let seeds = lock(&agents.seeds).clone();
    assert_eq!(seeds.len(), 2);
    assert_eq!(seeds[1], Vec::<Said>::new());
}

/// A full context inside a turn is the driver's to carry on from: the
/// chapter the person is in stays open, with no note written and no new
/// marker, and the driver continues without a wake of its own (BRO-150).
#[tokio::test]
async fn a_full_context_mid_turn_keeps_the_chapter_open() {
    let (boundary, handed) = crate::driver::ChapterBoundary::new();
    let mut turn = vec![Update::Chapter { boundary }];
    turn.extend(saying("one", "Still on it."));
    let agents = Fake::answering(Scripted::turns(vec![turn]), note_json("Crane jam"));
    let room = room("chapter-full-context", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "keep going", None, None).await.unwrap();
    settled(&room, "ada", 3).await;

    assert_eq!(handed.await.unwrap(), None);
    assert_eq!(
        kinds(&tape(&room, "ada")),
        ["chapter", "user", "agent", "turn"]
    );
    let markers = markers(&room, "ada");
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].get("endedAt"), None);
    assert_eq!(markers[0].get("closedBy"), None);
}

/// What the fresh context is told about the conversation it is joining.
#[tokio::test]
async fn the_wake_block_carries_the_previous_chapters_note() {
    let agents = Fake::answering(
        Scripted::turns(vec![
            saying("one", "It jammed."),
            saying("two", "All clear."),
        ]),
        note_json("Crane jam"),
    );
    let room = room("chapter-wake", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "did it jam?", None, None).await.unwrap();
    settled(&room, "ada", 3).await;
    room.start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    room.prompt("ada", "and now?", None, None).await.unwrap();
    settled(&room, "ada", 6).await;

    let woken = lock(&agents.preambles)[1].clone();
    assert!(woken.contains("You are Ada."), "{woken}");
    assert!(woken.contains("fresh working context"), "{woken}");
    assert!(
        woken.contains(r#"The previous chapter, "Crane jam", ended"#),
        "{woken}"
    );
    assert!(woken.contains("Goal: Get the crane moving"), "{woken}");
    assert!(woken.contains("Open loops:\n- oil the winch"), "{woken}");
    assert!(
        woken.contains(r#"{"speaker":"user","text":"did it jam?"}"#),
        "{woken}"
    );
}

/// The DM's row of the room's one sweep is its chapters.
#[tokio::test]
async fn the_rooms_sweep_closes_a_stale_chapter_as_the_dms_idle_row() {
    let log = scratch("chapter-room-sweep");
    enrol(&log, &persona("ada"));
    let stale = now_ms() - 10 * 3_600_000;
    write_tape(
        &log,
        "ada",
        &[
            json!({"kind": "chapter", "id": "c-ada", "ts": stale, "backendId": "hotline"}),
            spoken("user", "u1", stale + 1_000, "did the crane jam?"),
            spoken("agent", "a1", stale + 2_000, "It jammed."),
        ],
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::answering(Scripted::new(Vec::new()), note_json("Crane jam")),
    );

    room.sweep(now_ms(), &mut HashMap::new()).await;

    assert_eq!(markers(&room, "ada")[0]["closedBy"], "idle");
}

/// The idle clock: a chapter nobody has said anything in for longer than the
/// room allows closes itself, and one that is still warm is left alone.
#[tokio::test]
async fn the_idle_sweep_closes_a_stale_chapter_and_leaves_a_fresh_one() {
    let log = scratch("chapter-sweep");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let stale = now_ms() - 10 * 3_600_000;
    write_tape(
        &log,
        "ada",
        &[
            json!({"kind": "chapter", "id": "c-ada", "ts": stale, "backendId": "hotline"}),
            spoken("user", "u1", stale + 1_000, "did the crane jam?"),
            spoken("agent", "a1", stale + 2_000, "It jammed."),
        ],
    );
    write_tape(
        &log,
        "bob",
        &[
            json!({"kind": "chapter", "id": "c-bob", "ts": now_ms() - 60_000, "backendId": "hotline"}),
            spoken("user", "u2", now_ms() - 30_000, "morning"),
        ],
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::answering(Scripted::new(Vec::new()), note_json("Crane jam")),
    );

    room.sweep_chapters(&mut HashMap::new()).await;

    let closed = &markers(&room, "ada")[0];
    assert_eq!(
        closed["endedAt"],
        stale + 2_000,
        "the chapter ended when the conversation stopped, not when the sweep noticed"
    );
    assert_eq!(closed["title"], "Crane jam");
    assert_eq!(
        closed["note"],
        "Goal: Get the crane moving\nOutcome: It moved.\nOpen loops:\n- oil the winch\nFiles: crane.log"
    );
    assert_eq!(closed["status"], "in-progress");
    assert_eq!(closed["tags"], json!(["crane", "harbour"]));
    assert_eq!(closed["closedBy"], "idle");

    assert_eq!(
        markers(&room, "bob")[0].get("endedAt"),
        None,
        "a chapter that heard something a minute ago is not stale"
    );
}

/// Ada's chapter, last spoken in ten hours ago: past the room's eight.
fn quiet_room(name: &str, agents: Arc<Fake>) -> (Arc<Room>, i64) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    let stale = now_ms() - 10 * 3_600_000;
    write_tape(
        &log,
        "ada",
        &[
            json!({"kind": "chapter", "id": "c-ada", "ts": stale, "backendId": "hotline"}),
            spoken("user", "u1", stale + 1_000, "did the crane jam?"),
            spoken("agent", "a1", stale + 2_000, "It jammed."),
        ],
    );
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        agents,
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    (room, stale)
}

/// The message that ends a long quiet is the first line of the next chapter:
/// the stale one closes before it is written, ending when the conversation
/// stopped, so the reply is made in the new chapter's context and the divider
/// sits above the message rather than one message later.
#[tokio::test]
async fn a_message_after_the_idle_window_opens_the_next_chapter() {
    let (room, stale) = quiet_room(
        "chapter-arrival",
        Fake::answering(
            Scripted::turns(vec![saying("one", "Welcome back.")]),
            note_json("Crane jam"),
        ),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "back again", None, None).await.unwrap();
    settled(&room, "ada", 4).await;

    let events = tape(&room, "ada");
    assert_eq!(
        kinds(&events)[..6],
        ["chapter", "user", "agent", "chapter", "user", "agent"]
    );
    assert_eq!(events[0]["closedBy"], "idle");
    assert_eq!(
        events[0]["endedAt"],
        stale + 2_000,
        "the old chapter ended when the conversation stopped"
    );
    assert_eq!(events[3].get("endedAt"), None, "the new chapter is open");
    assert_eq!(events[4]["text"], "back again");
    assert_eq!(events[5]["text"], "Welcome back.");
}

/// A message being written holds the start gate, and the sweep leaves that
/// teammate to it: the message closes the chapter itself if it must, and
/// the sweep never writes a close over a line said while its note was out.
#[tokio::test]
async fn the_idle_sweep_leaves_a_chapter_to_a_message_being_written() {
    let (room, _) = quiet_room(
        "chapter-sweep-gate",
        Fake::answering(Scripted::new(Vec::new()), note_json("Crane jam")),
    );
    let gate = room.start_gate("ada");
    let held = gate.lock().await;
    room.sweep_chapters(&mut HashMap::new()).await;
    assert_eq!(markers(&room, "ada")[0].get("endedAt"), None);

    drop(held);
    room.sweep_chapters(&mut HashMap::new()).await;
    assert_eq!(markers(&room, "ada")[0]["closedBy"], "idle");
}

/// A message that ends a long quiet holds its teammate's start gate while
/// closing the chapter takes the policy lock, so an MCP sign-in rebuilding
/// the running main conversations takes no start gate under that lock: the
/// sign-in and the message both finish, each start as slow as a harness
/// coming up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_in_and_a_message_closing_a_quiet_chapter_both_finish() {
    let log = scratch("chapter-sign-in-quiet");
    let stale = now_ms() - 10 * 3_600_000;
    for who in ["ada", "bob"] {
        enrol(&log, &persona(who));
        write_tape(
            &log,
            who,
            &[
                json!({"kind": "chapter", "id": format!("c-{who}"), "ts": stale, "backendId": "hotline"}),
                spoken("user", &format!("u-{who}"), stale + 1_000, "old line"),
                spoken("agent", &format!("a-{who}"), stale + 2_000, "Old answer."),
            ],
        );
    }
    let starts = Arc::new(Semaphore::new(2));
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(saying("one", "Welcome back.")).slow_to_start(starts.clone())),
    );
    room.start("ada").await.unwrap();
    room.start("bob").await.unwrap();

    let signing = tokio::spawn({
        let room = room.clone();
        async move { room.signed_in_to_mcp().await }
    });
    let held = |who: &str| room.start_gate(who).try_lock().is_err();
    until("a rebuild holds a start gate", || {
        held("ada") || held("bob")
    })
    .await;
    let other = if held("ada") { "bob" } else { "ada" };
    let message = tokio::spawn({
        let room = room.clone();
        async move { room.prompt(other, "back again", None, None).await }
    });
    until("the message holds its teammate's gate", || held(other)).await;
    starts.add_permits(10);

    let deadline = Duration::from_secs(30);
    tokio::time::timeout(deadline, message)
        .await
        .expect("the message finishes")
        .unwrap()
        .unwrap();
    tokio::time::timeout(deadline, signing)
        .await
        .expect("the sign-in finishes")
        .unwrap();
    until(
        "every rebuild has let go of its gate and the policy lock",
        || !held("ada") && !held("bob") && room.policy_update_lock().try_lock().is_ok(),
    )
    .await;
    assert_eq!(markers(&room, other)[0]["closedBy"], "idle");
}

/// A person who asks for a fresh chapter is not kept waiting on a model: the
/// close is on the tape under the first message's title before the summariser
/// has said anything, and the note supersedes that marker when it lands.
#[tokio::test]
async fn closing_a_chapter_does_not_wait_for_its_note() {
    let agents = Fake::answering(
        Scripted::new(saying("one", "It jammed.")),
        note_json("Crane jam"),
    );
    agents
        .holding
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let room = room("chapter-note-later", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "did the crane jam?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 3).await;

    let closed = tokio::time::timeout(
        Duration::from_secs(5),
        room.start_fresh_chapter("ada", ChapterClose::User),
    )
    .await
    .expect("the close waited on the note")
    .unwrap();
    assert_eq!(closed.title.as_deref(), Some("did the crane jam?"));
    assert_eq!(closed.note, None);
    assert_eq!(markers(&room, "ada").len(), 1);

    agents.hold_note.notify_waiters();
    for _ in 0..100 {
        if markers(&room, "ada")[0]["title"] == "Crane jam" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let marker = &markers(&room, "ada")[0];
    assert_eq!(marker["title"], "Crane jam");
    assert_eq!(marker["closedBy"], "user");
    assert!(marker["note"].as_str().unwrap().contains("Get the crane"));
    assert_eq!(
        markers(&room, "ada").len(),
        1,
        "the note rewrote the marker"
    );
}

/// A teammate nobody spoke to does not collect empty rules in its drawer.
#[tokio::test]
async fn a_chapter_nobody_spoke_in_closes_without_a_title() {
    let room = room("chapter-empty", Fake::new(Scripted::new(Vec::new())));
    assert!(
        room.start_fresh_chapter("ada", ChapterClose::User)
            .await
            .is_err(),
        "a teammate that never started has no chapter to close"
    );

    room.start("ada").await.unwrap();
    let closed = room
        .start_fresh_chapter("ada", ChapterClose::Agent)
        .await
        .unwrap();

    assert_eq!(closed.title, None);
    assert_eq!(closed.messages, 0);
    let marker = &markers(&room, "ada")[0];
    assert_eq!(marker["closedBy"], "agent");
    assert!(marker["endedAt"].is_i64());
    assert_eq!(marker.get("title"), None);
    assert_eq!(marker.get("status"), None);
}

/// No model, or a model that would not answer: the chapter closes anyway,
/// named after the thing that started it, and the transcript says what is
/// missing rather than leaving the next chapter to start cold in silence.
#[tokio::test]
async fn a_chapter_no_model_would_summarise_closes_titled_from_the_first_message() {
    let agents = Fake::new(Scripted::new(saying("one", "It jammed.")));
    let room = room("chapter-no-note", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "did the crane jam?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 3).await;

    let closed = room
        .start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert_eq!(closed.title.as_deref(), Some("did the crane jam?"));
    assert_eq!(closed.note, None);
    assert_eq!(closed.status, Some(ChapterStatus::Done));

    assert!(
        tape(&room, "ada")
            .iter()
            .all(|event| event["kind"] != "notice"),
        "a missing handoff is not the conversation's business"
    );
}

/// The last process's open cards are expired and the tape folded before the
/// room serves anything from it, and the index knows the tape afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn opening_the_room_expires_orphaned_cards_and_indexes_the_tape() {
    let log = scratch("settle");
    let ada = persona("ada");
    enrol(&log, &ada);
    let tape = StreamId::Tape("ada".into());
    log.append(
        &tape,
        &json!({"kind": "user", "id": "u1", "ts": 1, "text": "harbour"}),
    )
    .unwrap();
    log.append(
        &tape,
        &json!({"kind": "permission", "id": "p1", "ts": 2, "requestId": "req", "title": "read a file", "options": []}),
    )
    .unwrap();
    log.append(
        &tape,
        &json!({"kind": "user", "id": "u1", "ts": 1, "text": "harbour again"}),
    )
    .unwrap();

    let _room = Room::new(log.clone(), Arc::new(DeskKeys));

    let events = log.load(&tape);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["decision"], "expired");
    // Compacted: the superseded line is gone from the file itself.
    let lines =
        std::fs::read_to_string(crate::paths::transcript_segment_path(log.root(), "ada", 1))
            .unwrap();
    assert_eq!(lines.lines().count(), 2);
    let found = crate::store::search::search(log.root(), "ada", "harbour", None).unwrap();
    assert_eq!(found["hits"].as_array().unwrap().len(), 1);
}

/// The room stream is folded at startup too: a roster entry rewritten a
/// thousand times is one line afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn opening_the_room_compacts_the_room_stream() {
    let log = scratch("settle-room");
    enrol(&log, &persona("ada"));
    for hours in 1..=20 {
        log.append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "setting-chapterIdleHours", "ts": hours, "key": "chapterIdleHours", "value": hours}),
        )
        .unwrap();
    }
    let path = crate::paths::room_path(log.root());
    let before = std::fs::read_to_string(&path).unwrap().lines().count();

    let _room = Room::new(log.clone(), Arc::new(DeskKeys));

    let after = std::fs::read_to_string(&path).unwrap().lines().count();
    assert!(after < before, "{after} lines of {before}");
    assert_eq!(log.load(&StreamId::Room).len(), after);
    assert_eq!(crate::room::roster(&log).len(), 1);
}
