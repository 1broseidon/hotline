//! A teammate's asides as the window and the phone read them: the count on
//! its roster row and `aside.list`.

use super::*;

fn left(id: &str, ts: i64, heard: Option<i64>) -> Value {
    json!({
        "kind": "aside", "id": id, "ts": ts, "to": "bob",
        "from": { "thread": { "kind": "pair", "key": "ada~bob" } },
        "by": "ada", "text": format!("aside {id}"), "level": "note",
        "heard": heard,
    })
}

#[tokio::test]
async fn aside_list_reads_a_teammates_asides_newest_first_and_the_row_counts_the_unheard() {
    let (_root, log, port, _room) = core_door("aside-list");
    let asides = StreamId::Asides("bob".into());
    for event in [
        left("a1", 1, Some(5)),
        left("a2", 2, None),
        left("a3", 3, None),
    ] {
        let mut event = event;
        if event["heard"].is_null() {
            event.as_object_mut().unwrap().remove("heard");
        }
        log.append(&asides, &event).unwrap();
    }

    let mut socket = desk(port).await;
    ask(&mut socket, json!({ "id": 1, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let rows = snapshot["snapshot"].as_array().unwrap();
    let row = |id: &str| rows.iter().find(|row| row["persona"]["id"] == id).unwrap();
    assert_eq!(row("bob")["asides"], 2, "{snapshot}");
    assert!(
        row("ada").get("asides").is_none(),
        "absent when none: {snapshot}"
    );

    // The phone reads them as the window does.
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    for seat in [Seat::Phone, Seat::Owner] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &log,
            &Parts::default(),
            json!({ "id": 2, "cmd": "aside.list", "params": { "personaId": "bob" } }),
        )
        .await;
        let ids: Vec<&str> = answer["result"]
            .as_array()
            .unwrap_or_else(|| panic!("{answer}"))
            .iter()
            .map(|aside| aside["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a3", "a2", "a1"], "{answer}");
        assert_eq!(answer["result"][2]["heard"], 5);
        assert_eq!(answer["result"][0]["from"]["thread"]["key"], "ada~bob");
    }
    assert!(Seat::Phone.permits(&Command::AsideList {
        persona_id: "bob".into()
    }));
}

/// The person's reading is apart from the teammate's hearing: the row counts
/// what the person has not seen (`asidesNew`), and `aside.seen`, which the
/// phone may send too, marks every aside seen and leaves `heard` as it was.
#[tokio::test]
async fn aside_seen_marks_the_persons_reading_and_the_row_counts_the_new() {
    let (_root, log, port, _room) = core_door("aside-seen");
    let asides = StreamId::Asides("bob".into());
    let mut seen = left("a1", 1, Some(5));
    seen["seen"] = json!(6);
    log.append(&asides, &seen).unwrap();
    let mut unseen = left("a2", 2, None);
    unseen.as_object_mut().unwrap().remove("heard");
    log.append(&asides, &unseen).unwrap();

    let roster = |port| async move {
        let mut socket = desk(port).await;
        ask(&mut socket, json!({ "id": 1, "sub": { "view": "roster" } })).await;
        let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
        snapshot["snapshot"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["persona"]["id"] == "bob")
            .unwrap()
            .clone()
    };
    let bob = roster(port).await;
    assert_eq!(bob["asidesNew"], 1, "{bob}");
    assert_eq!(bob["asides"], 1, "{bob}");

    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "aside.seen", "params": { "personaId": "bob" } }),
    )
    .await;
    let answer = heard_where(&mut socket, |frame| frame["id"] == 2).await;
    assert!(answer.get("error").is_none(), "{answer}");
    let kept = crate::store::asides::of(&log, "bob");
    assert!(kept.iter().all(|aside| aside.seen.is_some()), "{kept:?}");
    assert_eq!(kept[0].seen, Some(6), "an aside seen before keeps its time");
    assert_eq!(kept[1].heard, None, "seeing is not hearing");
    let bob = roster(port).await;
    assert!(bob.get("asidesNew").is_none(), "absent when none: {bob}");
    assert_eq!(bob["asides"], 1, "{bob}");

    assert!(Seat::Phone.permits(&Command::AsideSeen {
        persona_id: "bob".into()
    }));
    let looped = |kind| Command::ThreadLoopIn {
        thread: ThreadId::new(kind, "s1"),
        persona_id: "bob".into(),
    };
    assert!(Seat::Phone.permits(&looped(ThreadKind::Side)));
    assert!(!Seat::Phone.permits(&looped(ThreadKind::Pair)));
}
