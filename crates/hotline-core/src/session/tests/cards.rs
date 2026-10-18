//! Cards: permissions and requests to the person, and what outlives a turn.

use super::*;

/// The card a permission writes, and the decision that supersedes it.
///
/// One tape event, written twice by the same id: a fold shows the decided card
/// where the pending one stood, which is what "non-dismissable, and then
/// answered" looks like in an append-only file.
#[tokio::test]
async fn a_permission_is_one_card_that_the_answer_supersedes() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![
        Update::Permission {
            request_id: "r1".to_string(),
            title: "Run rm -rf /".to_string(),
            options: vec![
                PermissionOption {
                    option_id: "once".to_string(),
                    name: "Allow once".to_string(),
                    kind: Some("allow_once".to_string()),
                },
                PermissionOption {
                    option_id: "never".to_string(),
                    name: "Deny".to_string(),
                    kind: None,
                },
            ],
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]);
    driver.gate = Some(gate.clone());
    // The driver is holding this request open, the way a live agent would.
    lock(&driver.waiting).push("r1".to_string());
    let room = room("permission", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "clean up", None, None).await.unwrap();
    gate.add_permits(1);
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "permission"]);
    assert_eq!(events[1]["id"], "perm:r1");
    assert_eq!(events[1]["title"], "Run rm -rf /");
    assert_eq!(events[1]["options"][0]["name"], "Allow once");
    assert!(events[1].get("decision").is_none(), "the card is live");

    room.answer_permission("ada", "r1", "once").await.unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "permission"], "one card, not two");
    assert_eq!(events[1]["decision"], "once");
    assert_eq!(events[1]["decidedOptionName"], "Allow once");

    // Nothing is behind that button now, so a second answer is refused rather
    // than quietly letting an agent through.
    assert!(room.answer_permission("ada", "r1", "never").await.is_err());
}

/// A card the turn ended without an answer to is settled, because the agent
/// has stopped waiting and a button with nothing behind it is a lie.
#[tokio::test]
async fn a_card_the_turn_left_open_is_expired_when_the_turn_ends() {
    let driver = Scripted::new(vec![
        Update::Permission {
            request_id: "r1".to_string(),
            title: "Edit the harbour log".to_string(),
            options: vec![PermissionOption {
                option_id: "once".to_string(),
                name: "Allow once".to_string(),
                kind: None,
            }],
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]);
    lock(&driver.waiting).push("r1".to_string());
    let room = room("orphaned", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "tidy", None, None).await.unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "permission", "turn"]);
    assert_eq!(events[1]["decision"], "expired");
}

/// The agent's own id for the conversation is remembered on the teammate's
/// record — once, after a turn has completed on it, and one entry per backend.
#[tokio::test]
async fn a_fresh_sessions_id_is_remembered_after_the_turn_that_proves_it() {
    let agents = Fake::new(Scripted::turns(vec![
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
    ]));
    *lock(&agents.driver.session_id) = Some("s-1".to_string());
    let log = scratch("checkpoint");
    let mut ada = persona("ada");
    ada.backend_id = "cursor".to_string();
    // A checkpoint another harness left is not this one's to touch.
    ada.session_checkpoints = vec![SessionCheckpoint {
        backend_id: "opencode".to_string(),
        session_id: "elsewhere".to_string(),
    }];
    enrol(&log, &ada);
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("ada").await.unwrap();

    // Nothing is written before a turn has run on the session: some agents
    // issue an id they cannot reopen until a prompt has committed.
    assert_eq!(checkpoints(&room, "ada"), ["opencode/elsewhere"]);

    room.prompt("ada", "hello", None, None).await.unwrap();
    settled(&room, "ada", 2).await;
    for _ in 0..200 {
        if checkpoints(&room, "ada").len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        checkpoints(&room, "ada"),
        ["opencode/elsewhere", "cursor/s-1"]
    );
    assert_eq!(
        markers(&room, "ada")[0]["sessionId"],
        "s-1",
        "the marker carries the id a resume will point back at"
    );

    // Closing the chapter withdraws this backend's checkpoint and leaves the
    // other harness's conversation alone.
    room.start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert_eq!(checkpoints(&room, "ada"), ["opencode/elsewhere"]);
}

/// Resume reopens only the chapter immediately before: the interim closes as
/// a turning point, the new marker carries the earlier note, Hotline Agent is
/// seeded from that chapter's words, and the user lines said in the meantime
/// arrive as a nudge — never a line of the tape.
#[tokio::test]
async fn resume_reopens_the_previous_chapter_and_nudges_with_what_was_said_since() {
    let agents = Fake::new(Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]));
    let prompts = agents.driver.prompts.clone();
    let log = scratch("chapter-resume");
    enrol(&log, &persona("ada"));
    let t = now_ms();
    write_tape(
        &log,
        "ada",
        &[
            json!({
                "kind": "chapter", "id": "c1", "ts": t - 10_000, "backendId": "hotline",
                "sessionId": "s-old", "endedAt": t - 5_000, "title": "Crane jam",
                "note": "Goal: Get the crane moving", "status": "in-progress",
                "closedBy": "idle",
            }),
            spoken("user", "u1", t - 9_000, "did the crane jam?"),
            spoken("agent", "a1", t - 8_000, "It jammed."),
            json!({"kind": "chapter", "id": "c2", "ts": t - 1_000, "backendId": "hotline"}),
            spoken("user", "u2", t - 500, "and now?"),
        ],
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.start("ada").await.unwrap();

    let answered = TeammateTools::new(&room, "ada")
        .call("resume_chapter", &json!({}))
        .await
        .expect("resume_chapter is a teammate tool");
    let resumed: Value = serde_json::from_str(&answered).unwrap();
    assert_eq!(resumed["resumed"], json!(true));
    assert_eq!(resumed["title"], "Crane jam");

    let markers = markers(&room, "ada");
    assert_eq!(markers.len(), 3);
    assert_eq!(markers[1]["title"], "Back to: Crane jam");
    assert_eq!(markers[1]["closedBy"], "resume");
    assert_eq!(markers[1]["status"], "done");
    assert_eq!(markers[2]["resumedFrom"], "c1");
    assert_eq!(markers[2]["note"], "Goal: Get the crane moving");
    assert_eq!(markers[2]["sessionId"], "s-old");
    assert_eq!(
        markers[2].get("endedAt"),
        None,
        "the reopened marker is the open chapter"
    );

    let seeds = lock(&agents.seeds).clone();
    assert_eq!(seeds.len(), 2, "start, then the resume's restart");
    assert_eq!(
        words(seeds[1].clone()),
        vec![
            Said::User("did the crane jam?".to_string()),
            Said::Agent("It jammed.".to_string()),
        ]
    );

    let nudged = heard(&prompts, 1).await;
    assert!(
        nudged[0].contains("and now?"),
        "the nudge carries the interim user line: {}",
        nudged[0]
    );
    assert!(
        nudged[0].contains("<hotline_user_messages>"),
        "{}",
        nudged[0]
    );
    let users: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "user")
        .collect();
    assert_eq!(
        users.len(),
        2,
        "the nudge is Hotline's words, never a line of the tape"
    );

    let refused = room.resume_chapter("ada").await.unwrap_err();
    assert!(
        refused.contains("no previous chapter"),
        "a second resume is refused when the chapter immediately before closed by resume: {refused}"
    );
}

/// An ACP resume points the persona's checkpoint back at the previous
/// chapter's session, which is what the child's resume/load path will try.
#[tokio::test]
async fn resume_points_the_checkpoint_back_at_the_previous_chapters_session() {
    // No Turn: a turn would remember the scripted child's new session id
    // and overwrite the checkpoint this test is about.
    let agents = Fake::new(Scripted::new(Vec::new()));
    *lock(&agents.driver.session_id) = Some("s-new".to_string());
    let log = scratch("chapter-resume-checkpoint");
    let mut ada = persona("ada");
    ada.backend_id = "cursor".to_string();
    ada.session_checkpoints = vec![SessionCheckpoint {
        backend_id: "cursor".to_string(),
        session_id: "s-new".to_string(),
    }];
    enrol(&log, &ada);
    let t = now_ms();
    write_tape(
        &log,
        "ada",
        &[
            json!({
                "kind": "chapter", "id": "c1", "ts": t - 10_000, "backendId": "cursor",
                "sessionId": "s-old", "endedAt": t - 5_000, "title": "Crane jam",
                "note": "Goal: Get the crane moving", "closedBy": "idle",
            }),
            spoken("user", "u1", t - 9_000, "did the crane jam?"),
            json!({"kind": "chapter", "id": "c2", "ts": t - 1_000, "backendId": "cursor"}),
            spoken("user", "u2", t - 500, "and now?"),
        ],
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("ada").await.unwrap();
    room.resume_chapter("ada").await.unwrap();

    assert_eq!(checkpoints(&room, "ada"), ["cursor/s-old"]);
    let notices: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "notice")
        .collect();
    assert!(
        !notices
            .iter()
            .any(|notice| notice["text"].as_str().unwrap().contains("reopened")),
        "a scripted child does not restore, and the conversation carries on regardless: {notices:?}"
    );
}

/// The teammate's checkpoints as `backend/session`, oldest first.
fn checkpoints(room: &Room, persona_id: &str) -> Vec<String> {
    room::roster(&room.log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .map(|persona| {
            persona
                .session_checkpoints
                .into_iter()
                .map(|checkpoint| format!("{}/{}", checkpoint.backend_id, checkpoint.session_id))
                .collect()
        })
        .unwrap_or_default()
}

/// A pending `human_action` card's action id, once it has landed.
async fn pending_human(room: &Room, persona_id: &str) -> String {
    for _ in 0..200 {
        if let Some(id) = tape(room, persona_id).into_iter().find_map(|event| {
            (event["kind"] == "human_action" && event["status"] == "pending")
                .then(|| event["actionId"].as_str().map(str::to_string))
                .flatten()
        }) {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no pending human_action landed");
}

/// Waits for the `count`th delivery on a teammate's tape to have been heard
/// and its turn to have ended.
async fn heard_delivery(room: &Room, persona_id: &str, count: usize) -> Value {
    for _ in 0..300 {
        let tape = tape(room, persona_id);
        let deliveries: Vec<&Value> = tape
            .iter()
            .filter(|event| event["kind"] == "delivery")
            .collect();
        if let Some(delivery) = deliveries.get(count - 1)
            && delivery["receipt"] == "read"
            && tape
                .iter()
                .skip_while(|event| event["id"] != delivery["id"])
                .any(|event| event["kind"] == "turn")
        {
            return (*delivery).clone();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{persona_id} was never handed delivery {count}")
}

/// A teammate's own session asks and moves on: the tool returns at once,
/// and the person's answer, note and all, comes back as a delivery the
/// agent is handed on a turn of its own.
#[tokio::test]
async fn request_human_returns_at_once_and_the_answer_is_delivered() {
    let agents = Fake::new(Scripted::new(saying("a1", "on it")));
    let room = room("human-async", agents.clone());
    let asked = TeammateTools::new(&room, "ada")
        .call(
            "request_human",
            &json!({ "reason": "Which branch should I ship?" }),
        )
        .await
        .unwrap();
    assert!(asked.starts_with("Asked."), "{asked}");
    let action_id = pending_human(&room, "ada").await;
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .unwrap();
    assert_eq!(card["delivers"], true);

    room.answer_human(
        "ada",
        &action_id,
        HumanAnswer::Done,
        Some(" The release branch. ".to_string()),
    )
    .unwrap();
    assert!(
        room.answer_human("ada", &action_id, HumanAnswer::Declined, None)
            .is_err(),
        "a card is answered once"
    );

    let delivery = heard_delivery(&room, "ada", 1).await;
    assert_eq!(delivery["cause"]["kind"], "answer");
    assert_eq!(delivery["cause"]["actionId"], action_id.as_str());
    assert_eq!(delivery["cause"]["status"], "done");
    assert_eq!(delivery["cause"]["about"], "Which branch should I ship?");
    assert_eq!(delivery["text"], "The release branch.");
    let heard = lock(&agents.driver.prompts).clone();
    let last = heard.last().unwrap();
    assert!(
        last.contains("The person answered your request (\"Which branch should I ship?\")")
            && last.contains("The release branch."),
        "{last}"
    );
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .unwrap();
    assert_eq!(card["status"], "done");
    assert_eq!(card["note"], "The release branch.");
    assert_eq!(card["delivers"], true);
}

/// Nothing is parked on a card that delivers, so neither a cancelled turn nor
/// a restart takes it down: it is still answered, and still delivered.
#[tokio::test]
async fn a_card_that_delivers_outlives_a_cancel_and_a_restart() {
    let log = scratch("human-async-restart");
    enrol(&log, &persona("ada"));
    let room = Room::with_agents(
        log.clone(),
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(saying("a1", "on it"))),
    );
    room.start("ada").await.unwrap();
    TeammateTools::new(&room, "ada")
        .call("request_human", &json!({ "reason": "Tap the 2FA prompt" }))
        .await
        .unwrap();
    let action_id = pending_human(&room, "ada").await;
    room.cancel("ada").unwrap();
    room.stop("ada").unwrap();
    drop(room);

    for expired in
        crate::log::expire_orphaned_permissions(&log.load(&StreamId::Tape("ada".into())), now_ms())
    {
        log.append(&StreamId::Tape("ada".into()), &expired).unwrap();
    }
    let agents = Fake::new(Scripted::new(saying("a2", "thanks")));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    assert_eq!(pending_human(&room, "ada").await, action_id);
    room.answer_human("ada", &action_id, HumanAnswer::Declined, None)
        .unwrap();
    let delivery = heard_delivery(&room, "ada", 1).await;
    assert_eq!(delivery["cause"]["status"], "dismissed");
    assert!(
        lock(&agents.driver.prompts)
            .last()
            .unwrap()
            .contains("The person declined your request (\"Tap the 2FA prompt\").")
    );
}

/// A card that delivers and goes a day unanswered is taken down, and the
/// teammate that asked is told once.
#[tokio::test]
async fn a_card_left_a_day_is_expired_and_the_teammate_told() {
    let agents = Fake::new(Scripted::new(saying("a1", "ok")));
    let room = room("human-async-stale", agents.clone());
    room.write(
        "ada",
        &TranscriptEvent::HumanAction {
            id: "human:old".to_string(),
            ts: now_ms() - ASK_TTL - 1,
            action_id: "old".to_string(),
            reason: "Approve the deploy".to_string(),
            status: HumanActionStatus::Pending,
            note: None,
            delivers: Some(true),
            thread: None,
        },
    );
    room.write(
        "ada",
        &TranscriptEvent::HumanAction {
            id: "human:fresh".to_string(),
            ts: now_ms(),
            action_id: "fresh".to_string(),
            reason: "Approve the other deploy".to_string(),
            status: HumanActionStatus::Pending,
            note: None,
            delivers: Some(true),
            thread: None,
        },
    );

    room.expire_stale_asks(now_ms());
    room.expire_stale_asks(now_ms());

    let delivery = heard_delivery(&room, "ada", 1).await;
    assert_eq!(delivery["cause"]["status"], "expired");
    assert_eq!(delivery["cause"]["actionId"], "old");
    let tape = tape(&room, "ada");
    let status = |id: &str| {
        tape.iter()
            .find(|event| event["id"] == id)
            .map(|event| event["status"].clone())
            .unwrap()
    };
    assert_eq!(status("human:old"), "expired");
    assert_eq!(status("human:fresh"), "pending");
    assert_eq!(
        tape.iter()
            .filter(|event| event["kind"] == "delivery")
            .count(),
        1
    );
}

/// The agent asked the person; the answer flips the same card and unblocks
/// the tool with the sentence the agent reads. A colleague's side session
/// asks this way: it has no conversation of its own to be answered in.
#[tokio::test]
async fn request_human_writes_a_pending_card_and_the_answer_flips_it_to_done() {
    let room = room("human-done", Fake::new(Scripted::new(vec![])));
    let tools = TeammateTools::new(&room, "ada").for_peer();
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call("request_human", &json!({ "reason": "Tap the 2FA prompt" }))
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;
    room.answer_human("ada", &action_id, HumanAnswer::Done, Some("  ".to_string()))
        .unwrap();
    let text = waiting.await.unwrap().unwrap();
    assert_eq!(text, "The person answered.");

    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["id"], format!("human:{action_id}"));
    assert_eq!(card["status"], "done");
    assert_eq!(card["reason"], "Tap the 2FA prompt");
    assert!(
        room.answer_human("ada", &action_id, HumanAnswer::Declined, None)
            .is_err()
    );
}

/// An answered question carries the person's note on both the result and card.
#[tokio::test]
async fn an_answered_human_request_carries_the_note() {
    let room = room("human-answered-note", Fake::new(Scripted::new(vec![])));
    let tools = TeammateTools::new(&room, "ada").for_peer();
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call(
                    "request_human",
                    &json!({ "reason": "Which branch should I use?" }),
                )
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;
    room.answer_human(
        "ada",
        &action_id,
        HumanAnswer::Done,
        Some(" Use the release branch. ".to_string()),
    )
    .unwrap();
    let text = waiting.await.unwrap().unwrap();
    assert_eq!(
        text,
        "The person answered. They said: Use the release branch."
    );
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "done");
    assert_eq!(card["note"], "Use the release branch.");
}

/// The person's note reaches the agent word for word, on the sentence and
/// on the card, so a declined card says why and a card that asked a
/// question gets its answer.
#[tokio::test]
async fn a_declined_human_request_carries_the_note() {
    let room = room("human-declined", Fake::new(Scripted::new(vec![])));
    let tools = TeammateTools::new(&room, "ada").for_peer();
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call(
                    "request_human",
                    &json!({ "reason": "Enter the vault password" }),
                )
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;
    room.answer_human(
        "ada",
        &action_id,
        HumanAnswer::Declined,
        Some(" It is in the shared vault, use that. ".to_string()),
    )
    .unwrap();
    let text = waiting.await.unwrap().unwrap();
    assert_eq!(
        text,
        "The person declined. They said: It is in the shared vault, use that."
    );
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "dismissed");
    assert_eq!(card["note"], "It is in the shared vault, use that.");
}

/// The deadline is injectable so a test does not sit for ten minutes.
#[tokio::test]
async fn a_human_request_expires_when_nobody_answers() {
    let room = room("human-timeout", Fake::new(Scripted::new(vec![])));
    let text = room
        .request_human("ada", "Tap 2FA", Duration::from_millis(20))
        .await
        .unwrap();
    assert_eq!(text, "Nobody answered in ten minutes.");
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "expired");
}

/// A card left pending when the process died is a button nobody is behind.
#[tokio::test]
async fn a_human_action_left_pending_expires_when_the_room_opens() {
    let log = scratch("human-stale");
    enrol(&log, &persona("ada"));
    log.append(
        &StreamId::Tape("ada".into()),
        &json!({
            "kind": "human_action",
            "id": "human:stale",
            "ts": 1000,
            "actionId": "stale",
            "reason": "log in",
            "status": "pending",
        }),
    )
    .unwrap();
    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(Scripted::new(vec![])));
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is still on the tape");
    assert_eq!(card["status"], "expired");
    assert_eq!(card["id"], "human:stale");
}

/// A `persona` line with an empty id is a half-written record, not a
/// teammate: nothing could name its tape. The room opens over it rather than
/// taking the whole roster down when the startup fold reaches for that tape.
#[tokio::test]
async fn a_teammate_with_no_id_does_not_stop_the_room_opening() {
    let log = scratch("empty-id-settle");
    let mut nobody = persona("");
    nobody.name = "Nobody".to_string();
    enrol(&log, &nobody);
    enrol(&log, &persona("ada"));

    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
    );
    assert!(room.persona("ada").is_ok());
    assert!(room.persona("").is_err());
}

/// A permission raised inside a peer turn lives on the thread and nowhere
/// else, and no seat is shown one. A card that outlived the process that
/// received it is a button nobody is behind, on a stream nothing else
/// revisits.
#[tokio::test]
async fn a_card_left_open_on_a_thread_expires_when_the_room_opens() {
    let log = scratch("thread-settle");
    enrol(&log, &persona("ada"));
    let key = crate::paths::thread_key("ada", "bob").expect("a key for the pair");
    crate::log::thread::ensure(log.root(), &key).unwrap();
    log.append(
        &StreamId::Pair(key.clone()),
        &json!({
            "kind": "permission",
            "id": "perm:req-1",
            "ts": 1000,
            "requestId": "req-1",
            "title": "read a file",
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}],
        }),
    )
    .unwrap();

    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
    );
    let card = room
        .log
        .load(&StreamId::Pair(key))
        .into_iter()
        .find(|event| event["kind"] == "permission")
        .expect("the card is still on the thread");
    assert_eq!(card["decision"], "expired");
    assert_eq!(card["id"], "perm:req-1");
}

/// The tool that asked the person is inside the turn, so a cancelled turn is
/// an agent that has stopped listening. The card goes with it: a live button
/// that writes `done` for nobody is the failure these cards exist to avoid.
#[tokio::test]
async fn cancelling_a_turn_takes_the_card_it_asked_the_person_with() {
    let room = room("human-cancel", Fake::new(Scripted::new(vec![])));
    room.start("ada").await.unwrap();
    let tools = TeammateTools::new(&room, "ada").for_peer();
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call("request_human", &json!({ "reason": "Tap the 2FA prompt" }))
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;

    room.cancel("ada").unwrap();

    let released = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the tool was left parked on a turn that had been cancelled")
        .unwrap()
        .unwrap();
    assert_eq!(released, "Nobody answered in ten minutes.");
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "expired");
    assert!(
        room.answer_human("ada", &action_id, HumanAnswer::Done, None)
            .is_err(),
        "a card nobody is behind still took an answer"
    );
}

#[tokio::test]
async fn updates_wait_for_queued_work_and_release_the_room_after_failure() {
    let semaphore = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Message {
        kind: MessageKind::Agent,
        id: "answer".into(),
        text: "done".into(),
    }]);
    driver.gate = Some(semaphore.clone());
    let room = room("update-lease", Fake::new(driver));
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();
    room.prompt("ada", "queued", None, None).await.unwrap();
    assert!(
        room.prepare_restart()
            .unwrap_err()
            .contains("still working")
    );
    semaphore.add_permits(2);
    let held = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(held) = room.prepare_restart() {
                break held;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let before = tape(&room, "ada");
    assert!(
        room.prompt("ada", "must not be lost", None, None)
            .await
            .unwrap_err()
            .contains("restart")
    );
    assert!(room.start("ada").await.unwrap_err().contains("restart"));
    assert!(
        room.nudge("ada", "new work")
            .unwrap_err()
            .contains("restart")
    );
    assert_eq!(before, tape(&room, "ada"));
    // Failed installation/cancellation drops its lease, restoring normal admission.
    drop(held);
    semaphore.add_permits(1);
    room.prompt("ada", "retry", None, None).await.unwrap();
}

#[tokio::test]
async fn a_failed_turn_is_retried_on_the_same_line_without_writing_it_again() {
    let failed = vec![
        Update::Notice {
            level: NoticeLevel::Error,
            text:
                "Provider connection interrupted: The request could not complete over the network."
                    .into(),
        },
        Update::Turn {
            stop_reason: "failed".into(),
            usage: None,
        },
    ];
    let answered = vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "answer".into(),
            text: "It jammed on the third floor.".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ];
    let agents = Fake::new(Scripted::turns(vec![failed, answered]));
    let room = room("retry-failed", agents.clone());
    room.start("ada").await.unwrap();
    assert!(
        room.retry("ada")
            .await
            .unwrap_err()
            .contains("no failed turn"),
        "a turn that never failed was retried"
    );
    room.prompt("ada", "did the crane jam?", None, None)
        .await
        .unwrap();
    let after_failure = settled(&room, "ada", 3).await;
    assert_eq!(after_failure.last().unwrap()["stopReason"], "failed");

    room.retry("ada").await.unwrap();
    let events = settled(&room, "ada", 5).await;
    let users: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "user")
        .collect();
    assert_eq!(
        users.len(),
        1,
        "the retried line was written again: {events:?}"
    );
    assert_eq!(
        *lock(&agents.driver.retried),
        vec!["did the crane jam?".to_string()]
    );
    assert!(
        events
            .iter()
            .any(|event| event["text"] == "It jammed on the third floor.")
    );
    assert_eq!(events.last().unwrap()["stopReason"], "end_turn");
    assert!(
        room.retry("ada")
            .await
            .unwrap_err()
            .contains("no failed turn"),
        "a turn that answered was retried"
    );
}
