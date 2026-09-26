use super::*;
use serde_json::Value;

async fn send_between(room: &Arc<Room>, from: &str, to: &str, intent: &str) -> String {
    let answer = TeammateTools::new(room, from)
        .call(
            "message_teammate",
            &json!({"to":to, "message":"continue the exchange", "intent":intent}),
        )
        .await
        .unwrap();
    serde_json::from_str::<Value>(&answer).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .into()
}

fn cards(room: &Room, whose: &str) -> Vec<Value> {
    room.tape(whose)
        .into_iter()
        .filter(|v| v["kind"] == "exchange_paused")
        .collect()
}

#[tokio::test]
async fn every_pause_is_a_new_decision_on_both_tapes_and_stop_settles_the_queue() {
    let (room, _) = setup("successive-exchange-pauses");
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("ada", "bob").unwrap();
    let mut completed = Vec::new();
    let mut previous_ids = Vec::new();
    for cycle in 0..3 {
        // The preceding resume has already delivered the first request of
        // this cycle. Alternate both direction and intent through real tools.
        for i in 0..if cycle == 0 { 6 } else { 5 } {
            let (from, to, intent) = if i % 2 == 0 {
                ("ada", "bob", "ask")
            } else {
                ("bob", "ada", "handoff")
            };
            let id = send_between(&room, from, to, intent).await;
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

        // Nothing is lost or run while paused; resuming from either tape
        // releases the same queue. On the third pause the person stops it.
        let queued = send_between(&room, "ada", "bob", "handoff").await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            room.exchange_pair("ada~bob")
                .unwrap()
                .requests
                .last()
                .unwrap()
                .phase,
            Phase::Queued
        );
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
        if cycle < 2 {
            done(&room, &queued).await;
            completed.push(queued);
            assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 2);
        } else {
            room.resume_exchange("ada", "bob").unwrap();
            until(|| lock(&room.exchange_workers).is_empty()).await;
            let pair = room.exchange_pair("ada~bob").unwrap();
            let stopped = pair.requests.iter().find(|r| r.id == queued).unwrap();
            assert_eq!(stopped.phase, Phase::Stopped);
            assert!(stopped.failed);
            assert!(
                room.tape("bob")
                    .iter()
                    .all(|v| v["cause"]["requestId"] != queued)
            );
            assert!(
                room.tape("ada")
                    .iter()
                    .any(|v| v["id"] == format!("exchange-ended:{queued}"))
            );
            for whose in ["ada", "bob"] {
                assert_eq!(cards(&room, whose).last().unwrap()["status"], "stopped");
            }
        }
    }
    for id in completed {
        let pair = room.exchange_pair("ada~bob").unwrap();
        let request = pair.requests.iter().find(|r| r.id == id).unwrap();
        assert_eq!(request.phase, Phase::Done);
        assert_eq!(
            room.tape(&request.from)
                .iter()
                .filter(|v| v["cause"]["requestId"] == id)
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn an_operator_message_settles_legacy_pause_cards_without_touching_another_pair() {
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
    room.prompt("ada", "I am here; continue", None, None)
        .await
        .unwrap();
    for whose in ["ada", "bob"] {
        assert_eq!(cards(&room, whose)[0]["status"], "resumed");
        assert_eq!(cards(&room, whose)[0]["exchanges"], 12);
    }
    assert_eq!(cards(&room, "bob")[1]["status"], "pending");
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert!(!pair.paused);
    assert_eq!(pair.exchanges, 0);
}
