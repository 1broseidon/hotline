use super::*;

#[tokio::test]
async fn the_window_opens_on_a_tapes_last_lines_and_pages_back_to_the_first() {
    let (_root, log, port) = door("tape-window");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    let tape = StreamId::Tape(persona_id.clone());
    for n in 0..1_000 {
        log.append(
            &tape,
            &json!({ "kind": "user", "id": format!("m{n}"), "ts": n, "text": "line" }),
        )
        .unwrap();
    }

    ask(
        &mut socket,
        json!({ "id": 2, "sub": { "tape": persona_id } }),
    )
    .await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let lines = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(lines.len(), 400);
    assert_eq!(lines[0]["id"], "m600");
    assert_eq!(lines[399]["id"], "m999");

    let page = |id: i64, params: Value| json!({ "id": id, "cmd": "tape.page", "params": params });
    ask(
        &mut socket,
        page(3, json!({ "personaId": persona_id, "before": "m600" })),
    )
    .await;
    let older = answered(&mut socket, 3).await;
    assert_eq!(older["ok"], true, "{older}");
    let events = older["result"]["events"].as_array().unwrap();
    assert_eq!(
        (events.len(), &events[0]["id"], &events[399]["id"]),
        (400, &json!("m200"), &json!("m599"))
    );
    assert_eq!(older["result"]["more"], true);

    ask(
        &mut socket,
        page(4, json!({ "personaId": persona_id, "before": "m200" })),
    )
    .await;
    let first = answered(&mut socket, 4).await;
    assert_eq!(first["result"]["events"].as_array().unwrap().len(), 200);
    assert_eq!(first["result"]["more"], false);

    // A search hit far up: one page reaches it, with some context above.
    ask(
        &mut socket,
        page(
            5,
            json!({ "personaId": persona_id, "before": "m600", "through": "m50" }),
        ),
    )
    .await;
    let reached = answered(&mut socket, 5).await;
    let events = reached["result"]["events"].as_array().unwrap();
    assert_eq!(events[0]["id"], "m10");
    assert_eq!(events.last().unwrap()["id"], "m599");
    assert_eq!(reached["result"]["more"], true);

    // A line the tape no longer holds ends the window rather than failing it.
    ask(
        &mut socket,
        page(6, json!({ "personaId": persona_id, "before": "gone" })),
    )
    .await;
    let gone = answered(&mut socket, 6).await;
    assert_eq!(gone["result"], json!({ "events": [], "more": false }));
}

#[test]
fn an_owner_device_opens_a_tape_on_the_same_window_as_the_desk() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let tape = StreamId::Tape("ada".into());
    for n in 0..1_000 {
        log.append(
            &tape,
            &json!({ "kind": "user", "id": format!("m{n}"), "ts": n, "text": "line" }),
        )
        .unwrap();
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let lines = snapshot_for_seat(&log, &tape, seat, false);
        assert_eq!((lines.len(), &lines[0]["id"]), (400, &json!("m600")));
    }
    assert_eq!(
        snapshot_for_seat(&log, &tape, Seat::Phone, false).len(),
        200
    );
}

#[test]
fn a_long_turn_of_steps_still_opens_a_tape_on_the_last_message() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let tape = StreamId::Tape("ada".into());
    log.append(
        &tape,
        &json!({ "kind": "user", "id": "ask", "ts": 0, "text": "go" }),
    )
    .unwrap();
    for n in 1..=500 {
        log.append(
            &tape,
            &json!({ "kind": "thought", "id": format!("t{n}"), "ts": n, "text": "hm" }),
        )
        .unwrap();
    }
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let lines = snapshot_for_seat(&log, &tape, seat, false);
        assert_eq!(lines[0]["id"], "ask", "{seat:?}");
        assert_eq!(lines.last().unwrap()["id"], "t500", "{seat:?}");
    }
}

/// A room whose settings were written first, then buried under 2,000 other
/// events: newest-200 and 512 KB would cut every one of them.
fn a_room_with_buried_settings(log: &Log) -> Vec<(&'static str, Value)> {
    let room = StreamId::Room;
    let setting = |id: &str, value: Value| json!({ "kind": "setting", "id": id, "value": value });
    let early = [
        ("voice", json!({ "mode": "local", "speaker": "old" })),
        (
            "mcpServers",
            json!([{ "id": "ketch", "type": "stdio", "name": "ketch", "command": "ketch", "args": [] }]),
        ),
        ("enabledModels", json!({ "anthropic": ["opus"] })),
        ("chapterIdleHours", json!(4)),
        ("gone", json!(true)),
    ];
    for (id, value) in &early {
        log.append(&room, &setting(id, value.clone())).unwrap();
    }
    let filler = "x".repeat(4096);
    for n in 0..2_000 {
        let kind = if n % 2 == 0 {
            "exchange_pair"
        } else {
            "schedule"
        };
        log.append(
            &room,
            &json!({ "kind": kind, "id": format!("e{n}"), "ts": n, "note": filler }),
        )
        .unwrap();
    }
    // Rewritten late: the fold keeps its first slot, far behind the window.
    let voice = json!({ "mode": "local", "speaker": "new" });
    log.append(&room, &setting("voice", voice.clone())).unwrap();
    log.append(
        &room,
        &json!({ "kind": "setting", "id": "gone", "deleted": true }),
    )
    .unwrap();
    vec![
        ("voice", voice),
        // Servers are sent as the room always sends them: public, no env.
        (
            "mcpServers",
            crate::mcp::public_room_event(setting("mcpServers", early[1].1.clone()))["value"]
                .clone(),
        ),
        ("enabledModels", early[2].1.clone()),
        ("chapterIdleHours", early[3].1.clone()),
    ]
}

#[test]
fn a_phones_room_snapshot_carries_every_live_setting_past_the_window() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let live = a_room_with_buried_settings(&log);
    let room = StreamId::Room;
    let lines = snapshot_for_seat(&log, &room, Seat::Phone, false);

    let settings: Vec<&Value> = lines.iter().filter(|e| e["kind"] == "setting").collect();
    assert_eq!(settings.len(), live.len(), "{settings:?}");
    for (id, value) in &live {
        let sent = settings.iter().find(|e| e["id"] == *id).unwrap();
        assert_eq!(&sent["value"], value, "{id}");
    }
    assert!(!settings.iter().any(|e| e["id"] == "gone"));

    // The rest is still the newest window, cut to budget, and the phone reads
    // no exchange bookkeeping.
    let rest: Vec<&Value> = lines.iter().filter(|e| e["kind"] != "setting").collect();
    assert!(rest.iter().all(|e| e["kind"] == "schedule"));
    assert!(!rest.is_empty() && rest.len() <= 200, "{}", rest.len());
    assert_eq!(rest.last().unwrap()["id"], "e1999");
    let bytes = |events: &[&Value]| events.iter().map(|e| e.to_string().len()).sum::<usize>();
    assert!(bytes(&rest) <= 512 * 1024, "{}", bytes(&rest));
    assert!(bytes(&settings) < 4 * 1024, "{}", bytes(&settings));
}

#[test]
fn the_desk_and_an_owner_desk_still_get_the_whole_room() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    a_room_with_buried_settings(&log);
    let room = StreamId::Room;
    let whole: Vec<Value> = log
        .load(&room)
        .into_iter()
        .map(crate::mcp::public_room_event)
        .collect();
    assert_eq!(whole.len(), 2_005);
    for seat in [Seat::Desk, Seat::Owner] {
        assert_eq!(
            snapshot_for_seat(&log, &room, seat, false),
            whole,
            "{seat:?}"
        );
    }
}
