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
