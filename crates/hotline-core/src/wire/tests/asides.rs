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
