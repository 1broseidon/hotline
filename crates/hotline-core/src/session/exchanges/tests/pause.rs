use super::*;
use serde_json::Value;

/// `from` sends `to` an item that wants work, and `to` takes it at once:
/// the request the work runs as.
async fn send_between(room: &Arc<Room>, from: &str, to: &str) -> String {
    let sent = TeammateTools::new(room, from)
        .call(
            "send",
            &json!({"to":[to], "title":"continue the exchange", "wants":"work"}),
        )
        .await
        .unwrap();
    let item = serde_json::from_str::<Value>(&sent).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    TeammateTools::new(room, to)
        .call(
            "update_inbox",
            &json!({"items":[{"id": item, "state": "taken"}]}),
        )
        .await
        .unwrap();
    crate::store::inbox::of(&room.log, to)
        .into_iter()
        .find(|kept| kept.id == item)
        .and_then(|kept| kept.request)
        .unwrap()
}

fn cards(room: &Room, whose: &str) -> Vec<Value> {
    room.tape(whose)
        .into_iter()
        .filter(|v| v["kind"] == "exchange_paused")
        .collect()
}

#[tokio::test]
async fn every_pause_is_a_new_decision_on_both_tapes_and_stop_settles_it() {
    let (room, _) = setup("successive-exchange-pauses");
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("ada", "bob").unwrap();
    let mut completed = Vec::new();
    let mut previous_ids = Vec::new();
    for cycle in 0..3 {
        // Each item and each result counts: six pieces of work, both ways,
        // reach the cap.
        for i in 0..6 {
            let (from, to) = if i % 2 == 0 {
                ("ada", "bob")
            } else {
                ("bob", "ada")
            };
            let id = send_between(&room, from, to).await;
            done(&room, &id).await;
            completed.push(id);
        }
        let pair = room.exchange_pair("ada~bob").unwrap();
        assert_eq!(pair.exchanges, 12);
        assert!(pair.paused);
        let mut current = Vec::new();
        for whose in ["ada", "bob"] {
            let tape_cards = cards(&room, whose);
            assert_eq!(
                tape_cards.len(),
                cycle + 1,
                "{whose}: a new pause must not replace an old decision"
            );
            assert!(tape_cards[..cycle].iter().all(|c| c["status"] == "resumed"));
            let card = tape_cards.last().unwrap();
            assert_eq!(card["status"], "pending");
            assert_eq!(card["exchanges"], 12);
            assert!(!previous_ids.contains(&card["id"]));
            current.push(card.clone());
        }
        // Nothing is sent while paused.
        let refused = TeammateTools::new(&room, "ada")
            .call("send", &json!({"to":["bob"], "title":"held"}))
            .await
            .unwrap_err();
        assert!(refused.contains("paused"), "{refused}");
        let mut updates = [
            room.log.subscribe(&StreamId::Tape("ada".into())),
            room.log.subscribe(&StreamId::Tape("bob".into())),
        ];
        let status = if cycle == 2 {
            room.stop_exchange("bob", "ada").unwrap();
            "stopped"
        } else {
            room.resume_exchange("bob", "ada").unwrap();
            "resumed"
        };
        for ((whose, before), updates) in ["ada", "bob"].into_iter().zip(&current).zip(&mut updates)
        {
            let after = cards(&room, whose).pop().unwrap();
            assert_eq!(after["id"], before["id"]);
            assert_eq!(after["status"], status);
            assert_eq!(
                after["exchanges"], 12,
                "settlement keeps the count that prompted the decision"
            );
            assert_eq!(after["ts"], before["ts"]);
            let mut notified = false;
            while let Ok(event) = updates.try_recv() {
                notified |= event["id"] == before["id"] && event["status"] == status;
            }
            assert!(
                notified,
                "{whose}: the mounted tape must hear the settlement"
            );
            previous_ids.push(before["id"].clone());
        }
        assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 0);
    }
    for id in completed {
        let pair = room.exchange_pair("ada~bob").unwrap();
        let request = pair.requests.iter().find(|r| r.id == id).unwrap();
        assert_eq!(request.phase(), Phase::Done);
        assert!(result(&room, &id).is_some(), "{id} came back");
    }
}

#[tokio::test]
async fn a_persons_line_leaves_a_pause_and_keep_going_settles_legacy_cards_without_touching_another_pair()
 {
    let (room, _) = setup("legacy-exchange-pause-settlement");
    seed(
        &room,
        saved_request("legacy", Intent::Ask, Phase::Queued),
        12,
        true,
    );
    for (whose, other) in [("ada", "bob"), ("bob", "ada")] {
        room.write_value(
            whose,
            &json!({
                "kind":"exchange_paused", "id":format!("exchange-paused:ada~bob:{whose}"),
                "ts":1, "withPersonaId":other, "withName":other, "exchanges":12, "status":"pending"
            }),
        );
    }
    room.write_value(
        "bob",
        &json!({
            "kind":"exchange_paused", "id":"unrelated", "ts":2,
            "withPersonaId":"cara", "withName":"Cara", "exchanges":12, "status":"pending"
        }),
    );
    room.start("ada").await.unwrap();
    // A line about anything else is not the person's answer to the card.
    room.prompt("ada", "I am here; continue", None, None)
        .await
        .unwrap();
    for whose in ["ada", "bob"] {
        assert_eq!(cards(&room, whose)[0]["status"], "pending");
    }
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert!(pair.paused);
    assert_eq!(pair.exchanges, 12);
    room.resume_exchange("ada", "bob").unwrap();
    for whose in ["ada", "bob"] {
        assert_eq!(cards(&room, whose)[0]["status"], "resumed");
        assert_eq!(cards(&room, whose)[0]["exchanges"], 12);
    }
    assert_eq!(cards(&room, "bob")[1]["status"], "pending");
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert!(!pair.paused);
    assert_eq!(pair.exchanges, 0);
}
