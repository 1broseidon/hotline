use super::*;
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use std::time::Duration;

fn setup(name: &str) -> (Arc<Room>, Arc<Fake>) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let turns = (0..50)
        .map(|i| {
            vec![
                Update::Message {
                    kind: MessageKind::Agent,
                    id: format!("answer-{i}"),
                    text: format!("result-{i}"),
                },
                Update::Turn {
                    stop_reason: "end_turn".into(),
                    usage: None,
                },
            ]
        })
        .collect();
    let agents = Fake::new(Scripted::turns(turns));
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}
async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("exchange never reached the expected state");
}
async fn send(room: &Arc<Room>, intent: &str) -> String {
    let result = TeammateTools::new(room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":format!("work {intent}"),"intent":intent}),
        )
        .await
        .unwrap();
    serde_json::from_str::<serde_json::Value>(&result).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .into()
}
async fn done(room: &Room, id: &str) {
    until(|| {
        room.exchange_pair("ada~bob").is_some_and(|p| {
            p.requests
                .iter()
                .any(|r| r.id == id && r.phase == Phase::Done)
        })
    })
    .await;
}
fn saved_request(id: &str, intent: Intent, phase: Phase) -> Request {
    Request {
        id: id.into(),
        from: "ada".into(),
        to: "bob".into(),
        message: "saved work".into(),
        intent,
        phase,
        reply: String::new(),
        failed: false,
        reply_counted: false,
        request_counted: false,
        inline: false,
        started: false,
    }
}
fn seed(room: &Room, request: Request, count: i64, paused: bool) {
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: count,
        paused,
        requests: vec![request],
    })
    .unwrap();
}

#[tokio::test]
async fn handoff_uses_main_context_and_returns_the_matching_request_without_another_tool_call() {
    let (room, agents) = setup("handoff-context-result");
    room.write_value(
        "bob",
        &json!({"kind":"user","id":"secret","ts":1,"text":"recipient main context"}),
    );
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    done(&room, &id).await;
    let target = room
        .tape("bob")
        .into_iter()
        .find(|v| v["cause"]["kind"] == "handoff")
        .unwrap();
    assert_eq!(target["cause"]["requestId"], id);
    assert_eq!(target["receipt"], "read");
    let answer = room
        .tape("ada")
        .into_iter()
        .find(|v| v["cause"]["requestId"] == id)
        .unwrap();
    assert_eq!(answer["text"], "result-0");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["cause"]["threadKey"], "ada~bob");
    assert!(
        lock(&agents.seeds)[0]
            .iter()
            .any(|s| format!("{s:?}").contains("recipient main context"))
    );
    assert_eq!(room.log.load(&StreamId::Thread("ada~bob".into())).len(), 2);
}

#[tokio::test]
async fn default_ask_never_enters_or_reads_the_recipient_main_conversation() {
    let (room, agents) = setup("ask-isolated");
    room.write_value(
        "bob",
        &json!({"kind":"user","id":"private","ts":1,"text":"PRIVATE MAIN CONTEXT"}),
    );
    let response = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"bounded review"}),
        )
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    done(&room, response["requestId"].as_str().unwrap()).await;
    assert_eq!(response["intent"], "ask");
    assert!(room.tape("bob").iter().all(|v| v["kind"] != "delivery"));
    assert!(
        lock(&agents.seeds)[0]
            .iter()
            .all(|s| !format!("{s:?}").contains("PRIVATE MAIN CONTEXT"))
    );
    assert!(lock(&agents.preambles)[0].contains("replying privately"));
}

#[tokio::test]
async fn twelve_messages_include_replies_and_switching_intent_does_not_reset_the_brake() {
    let (room, _) = setup("cross-intent-brake");
    room.allow_sender("bob", "ada").unwrap();
    for i in 0..6 {
        let id = send(&room, if i % 2 == 0 { "ask" } else { "handoff" }).await;
        done(&room, &id).await;
    }
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.exchanges, 12);
    assert!(pair.paused);
    let queued = send(&room, "handoff").await;
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
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, &queued).await;
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 2);
    room.heard_from_person("bob");
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 0);
}

#[tokio::test]
async fn the_twelfth_request_keeps_its_reply_queued_until_keep_going() {
    let (room, _) = setup("brake-reply");
    seed(
        &room,
        saved_request("twelfth", Intent::Ask, Phase::Queued),
        11,
        false,
    );
    room.recover_queued_exchanges();
    until(|| room.exchange_pair("ada~bob").unwrap().requests[0].phase == Phase::Reply).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert!(pair.paused);
    assert_eq!(pair.exchanges, 12);
    assert!(!pair.requests[0].reply.is_empty());
    assert!(room.tape("ada").iter().all(|v| v["kind"] != "delivery"));
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, "twelfth").await;
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 1);
}

#[tokio::test]
async fn restart_preserves_a_paused_queue_and_unstarted_handoff_without_counting_twice() {
    let (old, _) = setup("restart-paused-exchange");
    old.allow_sender("bob", "ada").unwrap();
    let mut request = saved_request("waiting", Intent::Handoff, Phase::Running);
    request.request_counted = true;
    seed(&old, request, 12, true);
    let log = old.log.clone();
    drop(old);
    let agents = Fake::new(Scripted::new(vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "a".into(),
            text: "recovered result".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_queued_exchanges();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(agents.prompts().is_empty());
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Queued
    );
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, "waiting").await;
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().exchanges,
        1,
        "only the reply is new"
    );
}

#[tokio::test]
async fn restart_never_replays_started_work_and_delivers_an_explicit_failure() {
    let (room, agents) = setup("restart-started-exchange");
    let mut request = saved_request("started", Intent::Handoff, Phase::Running);
    request.started = true;
    request.request_counted = true;
    seed(&room, request, 1, false);
    room.recover_queued_exchanges();
    done(&room, "started").await;
    assert_eq!(agents.prompts().len(), 1, "only sender hears the failure");
    assert!(room.tape("bob").is_empty());
    let result = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery")
        .unwrap();
    assert_eq!(result["cause"]["status"], "failed");
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("inspect before retrying")
    );
}

#[tokio::test]
async fn stop_and_revocation_settle_saved_work_and_resume_cannot_resurrect_it() {
    let (room, agents) = setup("stop-revoke-exchange");
    seed(
        &room,
        saved_request("stopped", Intent::Ask, Phase::Queued),
        12,
        true,
    );
    room.stop_exchange("ada", "bob").unwrap();
    room.resume_exchange("ada", "bob").unwrap();
    room.recover_queued_exchanges();
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Stopped
    );
    seed(
        &room,
        saved_request("revoked", Intent::Handoff, Phase::Queued),
        12,
        true,
    );
    room.invalidate("bob").unwrap();
    room.resume_exchange("ada", "bob").unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Stopped
    );
    assert!(agents.prompts().is_empty());
    assert!(
        room.tape("ada")
            .iter()
            .any(|v| v["id"] == "exchange-ended:revoked")
    );
}

#[tokio::test]
async fn a_legacy_grant_requires_informed_handoff_approval_but_still_allows_ask() {
    let (room, _) = setup("legacy-handoff-grant");
    let mut target = room.persona("bob").unwrap();
    target.allowed_senders.push("ada".into());
    crate::room::append_persona(&room.log, &target).unwrap();
    let ask = send(&room, "ask").await;
    done(&room, &ask).await;
    let handoff = send(&room, "handoff").await;
    until(|| {
        room.tape("ada")
            .iter()
            .any(|v| v["kind"] == "permission" && v.get("decision").is_none())
    })
    .await;
    let card = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "permission" && v.get("decision").is_none())
        .unwrap();
    assert!(
        card["title"]
            .as_str()
            .unwrap()
            .contains("handoffs into its main conversation")
    );
    assert!(room.tape("bob").iter().all(|v| v["kind"] != "delivery"));
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow_always")
        .await
        .unwrap();
    done(&room, &handoff).await;
    let next = send(&room, "handoff").await;
    done(&room, &next).await;
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|v| v["kind"] == "permission")
            .count(),
        1
    );
}

#[tokio::test]
async fn handoff_waits_behind_the_persons_turn_and_stop_does_not_cancel_that_turn() {
    let log = scratch("handoff-turn-boundary");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let agents = Fake::new(
        Scripted::new(vec![Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        }])
        .gated(gate.clone()),
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.allow_sender("bob", "ada").unwrap();
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until(|| agents.prompts().len() == 1).await;
    let id = send(&room, "handoff").await;
    until(|| {
        room.tape("bob")
            .iter()
            .any(|v| v["cause"]["requestId"] == id)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts().len(), 1);
    assert_eq!(agents.cancel_count(), 0);
    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(
        agents.cancel_count(),
        0,
        "the person's work is not this handoff"
    );
    gate.add_permits(1);
    until(|| !room.mid_turn("bob")).await;
    assert_eq!(
        agents.prompts().len(),
        1,
        "stopped handoff never reaches the driver"
    );
}

#[tokio::test]
async fn concurrent_handoffs_keep_distinct_result_correlations() {
    let (room, _) = setup("handoff-distinct-results");
    room.allow_sender("bob", "ada").unwrap();
    let (one, two) = tokio::join!(send(&room, "handoff"), send(&room, "handoff"));
    assert_ne!(one, two);
    done(&room, &one).await;
    done(&room, &two).await;
    for id in [one, two] {
        let tape = room.tape("ada");
        let results: Vec<_> = tape
            .iter()
            .filter(|v| v["cause"]["requestId"] == id)
            .collect();
        assert_eq!(results.len(), 1);
        let pair = room.exchange_pair("ada~bob").unwrap();
        assert_eq!(
            results[0]["text"],
            pair.requests.iter().find(|r| r.id == id).unwrap().reply
        );
    }
}

#[tokio::test]
async fn recovery_and_the_result_worker_dispatch_a_saved_reply_only_once() {
    let (room, agents) = setup("reply-recovery-dedup");
    let mut request = saved_request("result-restart", Intent::Ask, Phase::Reply);
    request.reply = "saved answer".into();
    request.reply_counted = true;
    request.request_counted = true;
    seed(&room, request, 2, false);
    room.write_value(
        "ada",
        &json!({"kind":"delivery", "id":"exchange-result:result-restart",
        "ts": now_ms(), "receipt":"sent", "text":"saved answer", "cause": {
            "kind":"peer", "requestId":"result-restart", "personaId":"bob", "name":"Bob",
            "threadKey":"ada~bob", "status":"done", "about":"saved work"
        }}),
    );
    room.recover_exchanges().await;
    done(&room, "result-restart").await;
    until(|| !agents.prompts().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts().len(), 1);
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|v| v["kind"] == "delivery")
            .count(),
        1
    );
}
