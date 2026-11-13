//! A handoff's work thread, named to its sender: how a later handoff finds
//! it, and what holds up nothing else while the pair's requests go on.

use super::*;
use crate::contract::PeerStatus;
use serde_json::Value;

/// A handoff from Ada to Bob, named into `thread` when it is given.
async fn hand_off_to_bob(room: &Arc<Room>, message: &str, thread: Option<&str>) -> Value {
    hand_off(room, "ada", "bob", message, thread).await.unwrap()
}

fn id_of(sent: &Value) -> String {
    sent["requestId"].as_str().unwrap().to_string()
}

/// What `list_teammates` tells `caller` of `whom`.
async fn listed(room: &Arc<Room>, caller: &str, whom: &str) -> Value {
    let listed = TeammateTools::new(room, caller)
        .call("list_teammates", &json!({}))
        .await
        .unwrap();
    serde_json::from_str::<Value>(&listed).unwrap()["teammates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["personaId"] == whom)
        .cloned()
        .expect("the teammate is listed")
}

/// A sender finds the threads its handoffs opened where it looks for its
/// colleagues, and may name one by the work thread's own id as well as by a
/// request's; the recipient is not told of them as its own to continue.
#[tokio::test]
async fn a_handoffs_thread_is_listed_for_its_sender_and_named_by_its_own_id_too() {
    let (room, _) = setup("handoff-threads-listed");
    room.allow_sender("bob", "ada").unwrap();
    let first = hand_off_to_bob(&room, "Fix the build.", None).await;
    let first_id = id_of(&first);
    done(&room, &first_id).await;
    let side = work_thread(&room, &first_id);

    let row = listed(&room, "ada", "bob").await;
    assert_eq!(
        row["handoffThreads"],
        json!([{"thread": first_id, "sideThreadId": side, "phase": "done", "lastRequestId": first_id}])
    );
    assert!(
        listed(&room, "bob", "ada")
            .await
            .get("handoffThreads")
            .is_none()
    );

    for name in [side.clone(), format!("side:{side}")] {
        let more = hand_off_to_bob(&room, "And the lint.", Some(&name)).await;
        assert_eq!(more["thread"], first_id.as_str(), "{name}");
        done(&room, &id_of(&more)).await;
        assert_eq!(work_thread(&room, &id_of(&more)), side);
    }
}

/// A colleague's answer tells its sender the request it answers, the name a
/// later handoff passes to carry that work on; a failure does not.
#[test]
fn a_result_names_the_request_it_answers() {
    let cause = |status| DeliveryCause::Peer {
        request_id: Some("req-1".into()),
        persona_id: "bob".into(),
        name: "Bob".into(),
        thread_key: "ada~bob".into(),
        status,
        about: "Fix the build.".into(),
        answer: None,
    };
    for said in ["It builds now.", ""] {
        let heard = peers::delivery_wire(&cause(PeerStatus::Done), said);
        assert!(heard.ends_with(" Its requestId is req-1."), "{heard}");
    }
    let failed = peers::delivery_wire(&cause(PeerStatus::Failed), "It broke.");
    assert!(!failed.contains("req-1"), "{failed}");
}

/// Ada's handoff waits on the person's first-contact card, on a task of its
/// own: meanwhile Bob's question to Ada, which needs no card, is answered,
/// and the handoff goes on once the person allows it.
#[tokio::test]
async fn a_request_waiting_on_its_card_holds_up_nothing_else_between_the_two() {
    let (room, _) = setup("card-holds-up-nothing");
    room.allow_sender("ada", "bob").unwrap();
    let handoff = id_of(&hand_off_to_bob(&room, "Fix the build.", None).await);
    let card = || {
        room.tape("ada")
            .into_iter()
            .find(|v| v["kind"] == "permission" && v.get("decision").is_none())
    };
    until("the handoff waits on the person's card", || {
        card().is_some()
    })
    .await;

    let asked = TeammateTools::new(&room, "bob")
        .call(
            "message_teammate",
            &json!({"to":"ada","message":"Which branch?","intent":"ask"}),
        )
        .await
        .unwrap();
    let asked = serde_json::from_str::<Value>(&asked).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    done(&room, &asked).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    let waiting = pair.requests.iter().find(|r| r.id == handoff).unwrap();
    assert_eq!(
        waiting.phase(),
        Phase::Queued,
        "the handoff still waits on its card"
    );

    let card = card().unwrap();
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow_session")
        .await
        .unwrap();
    done(&room, &handoff).await;
}

/// A thread between two teammates is live while one answers the other, as
/// `thread.list` reads it, and both teammates' roster rows say so from the
/// first moment to the last (George's QA, 2026-11-12: a DM between agents
/// never showed among the running threads). The pair has no agent of its
/// own, so the roster row is the only word a window hears when it starts and
/// when it ends; nothing else on the wire moves for an ask.
#[tokio::test]
async fn a_pair_answering_an_ask_is_live_in_the_list_and_on_both_roster_rows() {
    let log = scratch("pair-answering-roster");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let agents = Fake::new(
        Scripted::turns(vec![vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "answer".into(),
                text: "It is paused.".into(),
            },
            Update::Turn {
                stop_reason: "end_turn".into(),
                usage: None,
            },
        ]])
        .gated(gate.clone()),
    );
    let room = Room::with_agents(log.clone(), Arc::new(DeskKeys), agents.clone());
    let key = crate::paths::thread_key("ada", "bob").unwrap();
    assert_eq!(room.working_pairs("ada"), Vec::<String>::new());

    let id = send(&room, "ask").await;
    until("bob is answering", || {
        room.exchange_pair(&key)
            .is_some_and(|pair| pair.requests[0].phase() == Phase::Running)
    })
    .await;
    let store = crate::thread::ThreadStore::new(&log);
    let listed = |persona: &str| {
        store
            .list(persona)
            .into_iter()
            .find(|thread| thread.id == ThreadId::pair(&key))
            .map(|thread| thread.state)
    };
    assert_eq!(listed("ada"), Some(crate::thread::ThreadState::Live));
    assert_eq!(listed("bob"), Some(crate::thread::ThreadState::Live));
    assert_eq!(room.working_pairs("ada"), std::slice::from_ref(&key));
    assert_eq!(room.working_pairs("bob"), std::slice::from_ref(&key));

    gate.add_permits(10);
    done(&room, &id).await;
    assert_eq!(listed("ada"), Some(crate::thread::ThreadState::Parked));
    assert_eq!(room.working_pairs("ada"), Vec::<String>::new());
    assert_eq!(room.working_pairs("bob"), Vec::<String>::new());
}
