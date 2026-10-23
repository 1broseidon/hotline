use super::*;

fn scheduled(
    id: &str,
    persona_id: &str,
    kind: crate::contract::ScheduleKind,
    next_at: i64,
) -> crate::contract::ScheduledJob {
    use crate::contract::ScheduleKind;
    crate::contract::ScheduledJob {
        id: id.to_string(),
        persona_id: persona_id.to_string(),
        kind,
        when: (kind == ScheduleKind::Schedule).then_some(next_at),
        every: (kind == ScheduleKind::Loop).then_some(60_000),
        prompt: format!("{id}, please"),
        quiet: None,
        reminder: None,
        goal: None,
        until: None,
        times: None,
        without: None,
        operator_created: true,
        next_at,
        created_at: 1_700_000_000_000,
        last_post: None,
        ended_at: None,
    }
}

#[tokio::test]
async fn a_teammates_schedules_are_the_whole_list_on_opening_and_again_on_each_change() {
    use crate::contract::ScheduleKind;
    let (_root, log, port) = door("schedules-view");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap();

    // Nothing scheduled is an answer, and it is said: `ok`, then an empty
    // list.
    ask(
        &mut socket,
        json!({ "id": 3, "sub": { "schedules": ada_id } }),
    )
    .await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 3, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 3, "snapshot": [] })
    );

    let once = scheduled(
        "job-once",
        ada_id,
        ScheduleKind::Schedule,
        1_700_000_900_000,
    );
    room::append_schedule(&log, &once).unwrap();
    let first = heard(&mut socket).await;
    assert_eq!(
        first,
        json!({ "sub": 3, "snapshot": [{
            "id": "job-once",
            "personaId": ada_id,
            "name": "job-once, please",
            "kind": "schedule",
            "prompt": "job-once, please",
            "when": 1_700_000_900_000_i64,
            "nextAt": 1_700_000_900_000_i64,
        }] })
    );

    let mut sweep = scheduled("job-loop", ada_id, ScheduleKind::Loop, 1_700_000_100_000);
    sweep.quiet = Some(true);
    room::append_schedule(&log, &sweep).unwrap();
    let both = heard(&mut socket).await;
    let ids: Vec<&str> = both["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["job-loop", "job-once"], "{both}");
    assert_eq!(both["snapshot"][0]["every"], 60_000);
    assert_eq!(both["snapshot"][0]["quiet"], true);
    assert!(both["snapshot"][0].get("when").is_none(), "{both}");

    // Another teammate's job changes nothing here, so nothing is sent: the
    // next frame is the loop's next run moving, once.
    room::append_schedule(
        &log,
        &scheduled("job-bob", bob_id, ScheduleKind::Loop, 1_700_000_050_000),
    )
    .unwrap();
    sweep.next_at += 60_000;
    room::append_schedule(&log, &sweep).unwrap();
    let moved = heard(&mut socket).await;
    assert_eq!(moved["snapshot"].as_array().unwrap().len(), 2, "{moved}");
    assert_eq!(moved["snapshot"][0]["nextAt"], 1_700_000_160_000_i64);
    assert!(!moved.to_string().contains("job-bob"), "{moved}");

    // A job that ended is still listed, with its end and its last post:
    // its thread stays readable.
    let mut ended = once.clone();
    ended.last_post = Some(crate::contract::JobPost {
        at: 1_700_000_950_000,
        text: "The crane is fine.".to_string(),
    });
    ended.ended_at = Some(1_700_000_960_000);
    room::append_schedule(&log, &ended).unwrap();
    let over = heard(&mut socket).await;
    assert_eq!(over["snapshot"][1]["id"], "job-once", "{over}");
    assert_eq!(over["snapshot"][1]["endedAt"], 1_700_000_960_000_i64);
    assert_eq!(
        over["snapshot"][1]["lastPost"],
        json!({"at": 1_700_000_950_000_i64, "text": "The crane is fine."})
    );

    // A deleted job is a tombstone that names no teammate; the list without
    // it is the answer.
    room::tombstone_schedule(&log, "job-once").unwrap();
    let fired = heard(&mut socket).await;
    assert_eq!(fired["snapshot"].as_array().unwrap().len(), 1, "{fired}");
    assert_eq!(fired["snapshot"][0]["id"], "job-loop");

    // A teammate deleted takes its list with it, and the view ends.
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.delete", "params": { "id": ada_id } }),
    )
    .await;
    let (mut deleted, mut removed) = (None, None);
    while deleted.is_none() || removed.is_none() {
        let frame = heard(&mut socket).await;
        if frame["id"] == 4 {
            deleted = Some(frame);
        } else if frame["sub"] == 3 {
            removed = Some(frame);
        }
    }
    assert_eq!(deleted.unwrap()["ok"], true);
    assert_eq!(removed.unwrap(), json!({ "sub": 3, "removed": ada_id }));
}

#[tokio::test]
async fn a_teammate_the_room_does_not_hold_or_cannot_read_is_refused_rather_than_empty() {
    let (root, _log, port) = door("schedules-refused");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "sub": { "schedules": "nobody" } }),
    )
    .await;
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "id": 1,
            "ok": false,
            "error": "There is no teammate nobody.",
            "code": "unknown_teammate",
        })
    );

    // A room stream that is there but cannot be read has no answer to give.
    std::fs::create_dir_all(root.join("room.jsonl")).unwrap();
    ask(
        &mut socket,
        json!({ "id": 2, "sub": { "schedules": "ada" } }),
    )
    .await;
    let unreadable = heard(&mut socket).await;
    assert_eq!(unreadable["ok"], false, "{unreadable}");
    assert_eq!(unreadable["code"], "unreadable", "{unreadable}");

    // Neither refusal left a subscription behind to reuse the id.
    let _ = std::fs::remove_dir(root.join("room.jsonl"));
    create(&mut socket, 3, "Ada").await;
    ask(
        &mut socket,
        json!({ "id": 1, "sub": { "schedules": "nobody" } }),
    )
    .await;
    assert_eq!(heard(&mut socket).await["code"], "unknown_teammate");
}

/// Behind on the room stream, the view reads the list again rather than
/// missing a change; unable to read it later, it says so and ends, since a
/// list it cannot refresh is no longer one to show as current.
#[tokio::test]
async fn a_schedules_view_catches_up_after_falling_behind_and_ends_when_it_cannot_read() {
    use crate::contract::ScheduleKind;
    let root = scratch("schedules-lag");
    let log = Log::open(&root);
    log.append(
        &StreamId::Room,
        &json!({"kind": "persona", "id": "ada", "name": "Ada", "goal": "", "backendId": "hotline", "cwd": "/tmp", "mcpPolicy": {"mode": "none", "serverIds": []}, "sessionCheckpoints": [], "createdAt": 1, "updatedAt": 1}),
    )
    .unwrap();
    room::append_schedule(
        &log,
        &scheduled("job-1", "ada", ScheduleKind::Loop, 1_700_000_100_000),
    )
    .unwrap();

    let (events, room_events) = broadcast::channel(1);
    for n in 0..3 {
        events.send(json!({"kind": "setting", "id": n})).unwrap();
    }
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        call_owner: "desk".into(),
        pairing_cancel: tokio_util::sync::CancellationToken::new(),
        auth_attempts: Arc::default(),
        account_logins: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        roster_options: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let view = tokio::spawn(crate::wire::schedules::view(
        9,
        "ada".to_string(),
        log.clone(),
        room_events,
        Vec::new(),
        outbox,
    ));
    let caught_up: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    assert_eq!(caught_up["sub"], 9);
    assert_eq!(caught_up["snapshot"][0]["id"], "job-1", "{caught_up}");

    std::fs::remove_file(root.join("room.jsonl")).unwrap();
    std::fs::create_dir_all(root.join("room.jsonl")).unwrap();
    events
        .send(json!({"kind": "schedule", "id": "job-1", "deleted": true}))
        .unwrap();
    let ended: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    assert_eq!(ended["sub"], 9);
    assert_eq!(ended["code"], "unreadable", "{ended}");
    tokio::time::timeout(std::time::Duration::from_secs(5), view)
        .await
        .expect("the view ended")
        .unwrap();
}
