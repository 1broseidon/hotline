use super::*;

#[tokio::test]
async fn a_subscription_is_the_fold_and_then_every_event_that_lands_after_it() {
    let (_root, log, port) = door("subscribe");
    let mut socket = desk(port).await;

    let already = json!({ "kind": "setting", "id": "chapterIdleHours", "value": 2 });
    log.append(&StreamId::Room, &already).unwrap();

    ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 7, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "snapshot": [already] })
    );

    let later = json!({ "kind": "setting", "id": "theme", "value": "dark" });
    log.append(&StreamId::Room, &later).unwrap();
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "event": later })
    );

    ask(&mut socket, json!({ "id": 8, "unsub": 7 })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 8, "ok": true }));
}

#[tokio::test]
async fn the_roster_is_one_row_per_living_teammate_and_a_tombstone_takes_one_away() {
    let (_root, _log, port) = door("roster");
    let mut socket = desk(port).await;

    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;

    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let rows = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["persona"], ada);
    assert_eq!(rows[0]["session"]["state"], "idle");
    assert!(rows[0]["preview"].is_null(), "{snapshot}");
    assert_eq!(rows[1]["persona"], bob);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.delete", "params": { "id": bob["id"] } }),
    )
    .await;
    let removed = heard_where(&mut socket, |frame| frame["removed"].is_string()).await;
    assert_eq!(removed, json!({ "sub": 3, "removed": bob["id"] }));
}

#[tokio::test]
async fn pinning_orders_the_desks_pins_and_the_roster_rows_carry_their_slot() {
    let (_root, log, port) = door("pins");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let cy = create(&mut socket, 3, "Cy").await;
    let di = create(&mut socket, 4, "Di").await;

    ask(&mut socket, json!({ "id": 5, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert!(snapshot["snapshot"][0]["pin"].is_null(), "{snapshot}");

    let pin = |id: i64, who: &Value, slot: Value| json!({ "id": id, "cmd": "persona.pin", "params": { "id": who["id"], "slot": slot } });
    ask(&mut socket, pin(6, &ada, json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 6).await["result"],
        json!([ada["id"]])
    );
    let row = heard_where(&mut socket, |f| f["event"]["persona"]["id"] == ada["id"]).await;
    assert_eq!(row["event"]["pin"], 0, "{row}");

    // Inserting at a slot shifts the rest along; a slot past the end appends.
    ask(&mut socket, pin(7, &bob, json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 7).await["result"],
        json!([bob["id"], ada["id"]])
    );
    let row = heard_where(&mut socket, |f| {
        f["event"]["persona"]["id"] == ada["id"] && f["event"]["pin"] == 1
    })
    .await;
    assert_eq!(row["event"]["pin"], 1, "{row}");
    ask(&mut socket, pin(8, &cy, json!(9))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 8).await["result"],
        json!([bob["id"], ada["id"], cy["id"]])
    );

    // A fourth is refused and says why; moving a pinned one is not a fourth.
    ask(&mut socket, pin(9, &di, json!(0))).await;
    let refused = heard_where(&mut socket, |f| f["id"] == 9).await;
    assert_eq!(refused["ok"], false);
    assert_eq!(
        refused["error"],
        "A desk pins up to 3 teammates. Unpin one first."
    );
    ask(&mut socket, pin(10, &cy, json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 10).await["result"],
        json!([cy["id"], bob["id"], ada["id"]])
    );

    ask(&mut socket, pin(11, &bob, Value::Null)).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 11).await["result"],
        json!([cy["id"], ada["id"]])
    );
    let row = heard_where(&mut socket, |f| {
        f["event"]["persona"]["id"] == bob["id"] && f["event"]["pin"].is_null()
    })
    .await;
    assert!(row["event"]["pin"].is_null(), "{row}");
    assert_eq!(
        room::pinned_teammates(&log),
        vec![cy["id"].as_str().unwrap(), ada["id"].as_str().unwrap()]
    );

    ask(&mut socket, pin(12, &json!({ "id": "nobody" }), json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 12).await["error"],
        "There is no teammate nobody."
    );
}

#[tokio::test]
async fn deleting_a_pinned_teammate_frees_its_slot() {
    let (_root, log, port) = door("pins-delete");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    for (id, who) in [(3, &ada), (4, &bob)] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": "persona.pin", "params": { "id": who["id"], "slot": 9 } }),
        )
        .await;
        assert_eq!(
            heard_where(&mut socket, |f| f["id"] == id).await["ok"],
            true
        );
    }
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "persona.delete", "params": { "id": ada["id"] } }),
    )
    .await;
    assert_eq!(heard_where(&mut socket, |f| f["id"] == 5).await["ok"], true);
    assert_eq!(room::settings(&log)["pinnedTeammates"], json!([bob["id"]]));
}

#[test]
fn only_a_desk_or_owner_seat_may_pin() {
    let pin = Command::PersonaPin {
        id: "ada".to_string(),
        slot: Some(0),
    };
    assert!(Seat::Desk.permits(&pin));
    assert!(Seat::Owner.permits(&pin));
    assert!(!Seat::Phone.permits(&pin));
}

#[tokio::test]
async fn a_line_in_a_tape_is_the_rosters_preview_of_that_teammate() {
    let (_root, log, port) = door("roster-preview");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "user", "id": "u1", "ts": 5, "text": "morning" }),
    )
    .unwrap();

    let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(
        row["event"]["preview"],
        json!({ "from": "me", "text": "morning", "at": 5 })
    );
    assert_eq!(row["event"]["latest"], 5);
}

#[tokio::test]
async fn a_card_waiting_on_the_person_marks_the_row_until_it_is_answered() {
    let (_root, log, port) = door("roster-waiting");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(snapshot["snapshot"][0]["waiting"], false, "{snapshot}");

    let tape = StreamId::Tape(persona_id);
    let card = |decision: Option<&str>| {
        let mut card = json!({
            "kind": "permission", "id": "p1", "ts": 5, "requestId": "r1",
            "title": "Run make", "options": [{ "optionId": "allow", "name": "Allow" }],
        });
        if let Some(decision) = decision {
            card["decision"] = json!(decision);
        }
        card
    };
    log.append(&tape, &card(None)).unwrap();
    let asked = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(asked["event"]["waiting"], true, "{asked}");

    log.append(&tape, &card(Some("allow"))).unwrap();
    let answered = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(answered["event"]["waiting"], false, "{answered}");

    log.append(
        &tape,
        &json!({
            "kind": "human_action", "id": "h1", "ts": 6, "actionId": "a1",
            "reason": "Tap the 2FA prompt", "status": "pending",
        }),
    )
    .unwrap();
    let human = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(human["event"]["waiting"], true, "{human}");
}

#[tokio::test]
async fn a_card_waiting_in_a_side_thread_marks_the_row_though_the_tape_has_none() {
    let room = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("roster-thread-waiting", room.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(snapshot["snapshot"][0]["waiting"], false, "{snapshot}");

    room.thread_cards.lock().unwrap().insert(persona_id.clone());
    room.set_info(idle(&persona_id));
    let asked = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(asked["event"]["waiting"], true, "{asked}");
}

#[tokio::test]
async fn an_exchange_pause_updates_the_live_roster_until_resumed_or_stopped() {
    let (_root, log, port) = door("roster-exchange-pause");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let tape = StreamId::Tape(ada["id"].as_str().unwrap().to_string());
    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    for status in ["pending", "resumed", "pending", "stopped"] {
        log.append(
            &tape,
            &json!({
                "kind": "exchange_paused", "id": "pause-1", "ts": 5,
                "withPersonaId": "mack", "withName": "Mack", "exchanges": 12,
                "status": status,
            }),
        )
        .unwrap();
        let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
        assert_eq!(row["event"]["waiting"], status == "pending", "{row}");
    }
}

#[tokio::test]
async fn the_rosters_latest_is_the_last_message_ts_and_a_tool_does_not_move_it() {
    let (_root, log, port) = door("roster-latest");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "user", "id": "u1", "ts": 5, "text": "morning" }),
    )
    .unwrap();
    let first = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(first["event"]["latest"], 5);

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 8, "toolCallId": "c1",
            "title": "Read main.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let tool = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(tool["event"]["latest"], 5);
    assert!(tool["event"].get("activity").is_none(), "{tool}");

    log.append(
        &StreamId::Tape(persona_id),
        &json!({ "kind": "agent", "id": "a1", "ts": 9, "text": "on it" }),
    )
    .unwrap();
    let second = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(second["event"]["latest"], 9);
}

#[tokio::test(flavor = "current_thread")]
async fn the_roster_recovers_a_session_update_lost_to_lag() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("roster-session-lag", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap();
    quiet.set_info(thinking(ada_id));

    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    let initial = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let ada_row = initial["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["persona"]["id"] == ada_id)
        .unwrap();
    assert_eq!(ada_row["session"]["state"], "thinking");

    // No await on this single-threaded runtime: Ada's final update falls out
    // of the 16-slot channel before the roster can receive it. Only Bob's
    // updates remain, so replaying those cannot repair Ada's stale state.
    quiet.set_info(idle(ada_id));
    for _ in 0..32 {
        quiet.set_info(thinking(bob_id));
    }

    let recovered = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(recovered["sub"], 3);
    let rows = recovered["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (persona_id, state) in [(ada_id, "idle"), (bob_id, "thinking")] {
        let row = rows
            .iter()
            .find(|row| row["persona"]["id"] == persona_id)
            .unwrap();
        assert_eq!(row["session"]["state"], state);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_roster_tape_burst_reads_each_dirty_teammate_once() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-burst", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ids = [ada["id"].as_str().unwrap(), bob["id"].as_str().unwrap()];
    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    quiet.info_reads.lock().unwrap().clear();

    // Overrun both tape receivers without letting their pumps or the view
    // run. Only the final state matters, not the thousand nudges per row.
    for n in 0..1000 {
        for persona_id in ids {
            log.append(
                &StreamId::Tape(persona_id.to_string()),
                &json!({
                    "kind": "agent", "id": format!("a{n}"), "ts": n, "text": format!("line {n}")
                }),
            )
            .unwrap();
        }
    }
    for _ in 0..2 {
        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            heard_where(&mut socket, |frame| frame["event"].is_object()),
        )
        .await
        .unwrap();
        assert_eq!(frame["event"]["preview"]["text"], "line 999");
        assert_eq!(frame["event"]["latest"], 999);
    }
    let mut reads = quiet.info_reads.lock().unwrap().clone();
    reads.sort();
    let mut expected = ids.map(str::to_string);
    expected.sort();
    assert_eq!(reads, expected);
}

#[tokio::test(flavor = "current_thread")]
async fn a_roster_session_burst_refreshes_each_teammate_once() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("roster-session-burst", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ids = [ada["id"].as_str().unwrap(), bob["id"].as_str().unwrap()];
    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    quiet.info_reads.lock().unwrap().clear();
    for _ in 0..8 {
        for id in ids {
            quiet.set_info(thinking(id));
        }
    }
    for _ in 0..2 {
        let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
        assert_eq!(row["event"]["session"]["state"], "thinking");
    }
    let mut reads = quiet.info_reads.lock().unwrap().clone();
    reads.sort();
    let mut expected = ids.map(str::to_string);
    expected.sort();
    assert_eq!(reads, expected);
}

#[tokio::test]
async fn roster_updates_use_the_cached_persona_after_a_room_edit() {
    let quiet = Arc::new(Quiet::new());
    let (root, log, port) = door_with("roster-cached-persona", quiet.clone());
    let mut socket = desk(port).await;
    let mut ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();
    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    ada["kind"] = json!("persona");
    ada["name"] = json!("Ada updated");
    log.append(&StreamId::Room, &ada).unwrap();
    let updated = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(updated["event"]["persona"]["name"], "Ada updated");

    // Make another room read impossible; a session hint should need only
    // the retained persona and the tape tail, not the room file again.
    std::fs::rename(root.join("room.jsonl"), root.join("room.saved")).unwrap();
    quiet.set_info(thinking(&id));
    let row = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        heard_where(&mut socket, |frame| frame["event"].is_object()),
    )
    .await
    .unwrap();
    assert_eq!(row["event"]["persona"]["name"], "Ada updated");
    assert_eq!(row["event"]["session"]["state"], "thinking");
    std::fs::rename(root.join("room.saved"), root.join("room.jsonl")).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn room_lag_rebuilds_cached_personas_and_late_hints_cannot_restore_a_deletion() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-room-lag", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let mut bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap().to_string();
    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    bob["kind"] = json!("persona");
    bob["name"] = json!("Bob updated");
    log.append(&StreamId::Room, &bob).unwrap();
    log.append(
        &StreamId::Room,
        &json!({"kind": "persona", "id": ada_id, "deleted": true}),
    )
    .unwrap();
    for n in 0..1_200 {
        log.append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "noise", "value": n}),
        )
        .unwrap();
    }
    let recovered = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        heard_where(&mut socket, |frame| frame["snapshot"].is_array()),
    )
    .await
    .unwrap();
    let rows = recovered["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["persona"]["name"], "Bob updated");
    quiet.info_reads.lock().unwrap().clear();
    quiet.set_info(thinking(ada_id));
    log.append(
        &StreamId::Tape(ada_id.to_string()),
        &json!({"kind": "agent", "id": "late", "text": "late", "ts": 1}),
    )
    .unwrap();
    quiet.set_info(thinking(&bob_id));
    let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(row["event"]["persona"]["id"], bob_id);
    assert_eq!(*quiet.info_reads.lock().unwrap(), [bob_id]);
}

#[tokio::test]
async fn search_failures_are_wire_errors_instead_of_empty_results() {
    let (root, _log, port) = door("search-unavailable");
    let mut socket = desk(port).await;
    for (id, command) in [(1, "search.thread"), (2, "search.all")] {
        ask(
            &mut socket,
            json!({"id": id, "cmd": command,
            "params": {"personaId": "ada", "query": "harbour"}}),
        )
        .await;
        let answer = heard_where(&mut socket, |frame| frame["id"] == id).await;
        assert_eq!(answer["ok"], false);
        assert_eq!(
            answer["error"],
            "Search is unavailable right now. Please try again."
        );
        assert!(answer.get("result").is_none());
    }
    assert!(!paths::index_path(&root).exists());
}

#[tokio::test]
async fn a_tool_in_progress_is_the_rosters_activity_while_the_session_is_thinking() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-activity", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    quiet.set_info(thinking(&persona_id));
    let thinking_row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(thinking_row["event"]["session"]["state"], "thinking");
    assert!(
        thinking_row["event"].get("activity").is_none(),
        "{thinking_row}"
    );

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 10, "toolCallId": "c1",
            "title": "Read main.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let running = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(running["event"]["activity"], "Read main.rs");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 11, "toolCallId": "c1",
            "title": "Read main.rs", "status": "completed",
        }),
    )
    .unwrap();
    let done = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert!(done["event"].get("activity").is_none(), "{done}");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t2", "ts": 12, "toolCallId": "c2",
            "title": "Edit lib.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let next = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(next["event"]["activity"], "Edit lib.rs");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "turn", "id": "tu1", "ts": 13, "stopReason": "end_turn" }),
    )
    .unwrap();
    let ended = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert!(ended["event"].get("activity").is_none(), "{ended}");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t3", "ts": 14, "toolCallId": "c3",
            "title": "Run tests", "status": "in_progress",
        }),
    )
    .unwrap();
    let again = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(again["event"]["activity"], "Run tests");

    quiet.set_info(idle(&persona_id));
    let idle_row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(idle_row["event"]["session"]["state"], "idle");
    assert!(idle_row["event"].get("activity").is_none(), "{idle_row}");
}
