//! Two teammates reaching each other, driven by the scripted driver.
//!
//! Nothing here reaches a model. What is under test is what an item needs to
//! land in a colleague's inbox: nothing between equals or down in access, and
//! up in access the person's card, how long its answer lasts and whom it
//! covers; and the receipts a pair's thread from before the inbox still
//! moves.

use super::*;
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use crate::session::{new_id, now_ms};
use crate::test_support::until;
use serde_json::json;

/// A room with Ada and Bob enrolled, neither of them running.
fn room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    let mut bob = persona("bob");
    bob.name = "Bob".to_string();
    enrol(&log, &bob);
    Room::with_agents(log, Arc::new(DeskKeys), agents)
}

/// Ada with its own folders and Bob with the whole machine, so Ada's
/// requests to Bob go up in access and need the person's word, and Bob's to
/// Ada go down and need none.
fn up_room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
    reach_room(name, agents, None, Some(crate::contract::Reach::Machine))
}

fn reach_room(
    name: &str,
    agents: Arc<Fake>,
    ada_reach: Option<crate::contract::Reach>,
    bob_reach: Option<crate::contract::Reach>,
) -> Arc<Room> {
    let log = scratch(name);
    let mut ada = persona("ada");
    ada.name = "Ada".to_string();
    ada.reach = ada_reach;
    enrol(&log, &ada);
    let mut bob = persona("bob");
    bob.name = "Bob".to_string();
    bob.reach = bob_reach;
    enrol(&log, &bob);
    Room::with_agents(log, Arc::new(DeskKeys), agents)
}

async fn collaboration_card(room: &Room, persona_id: &str) -> Value {
    let card = || {
        room.tape(persona_id).into_iter().find(|event| {
            kind_of(event) == "permission"
                && event["requestId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(COLLAB_REQUEST_PREFIX))
                && event.get("decision").is_none()
        })
    };
    until("the collaboration card reaches the tape", || {
        card().is_some()
    })
    .await;
    card().unwrap()
}

fn permission_cards(room: &Room, persona_id: &str) -> usize {
    room.tape(persona_id)
        .iter()
        .filter(|event| kind_of(event) == "permission")
        .count()
}

/// `from` sends `to` an item with its tools, as an agent does, and where it
/// is kept: `<receiver>/<item id>`.
async fn ask(room: &Arc<Room>, from: &str, to: &str, message: &str) -> Result<String, String> {
    send_wanting(room, from, to, message, "answer").await
}

async fn send_wanting(
    room: &Arc<Room>,
    from: &str,
    to: &str,
    message: &str,
    wants: &str,
) -> Result<String, String> {
    let sent = TeammateTools::new(room, from)
        .call(
            "send",
            &json!({ "to": [to], "title": message, "message": message, "wants": wants }),
        )
        .await?;
    let receiver = room.teammate_named(to)?.id;
    Ok(format!(
        "{receiver}/{}",
        serde_json::from_str::<Value>(&sent).unwrap()["sent"][0]["id"]
            .as_str()
            .unwrap()
    ))
}

/// What the card decided for this item, once it has: `done` when it landed
/// in its receiver's inbox, `failed` with why when it was dropped unseen.
async fn answer(room: &Room, _whose: &str, kept: &str) -> Value {
    let (receiver, id) = kept.split_once('/').unwrap();
    let found = || {
        crate::store::inbox::of(&room.log, receiver)
            .into_iter()
            .find(|item| item.id == id)
            .filter(|item| !item.held || item.state == crate::contract::InboxState::Closed)
    };
    until("the card is decided", || found().is_some()).await;
    let item = found().unwrap();
    let name = room
        .persona(receiver)
        .map(|persona| persona.name)
        .unwrap_or_default();
    if item.held {
        json!({"cause": {"status": "failed", "name": name}, "text": item.reason})
    } else {
        json!({"cause": {"status": "done", "name": name}, "text": item.body})
    }
}

#[tokio::test]
async fn a_request_up_in_access_asks_the_person_and_deny_drops_it_unseen() {
    let room = up_room(
        "workspace-deny",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let id = ask(&room, "ada", "bob", "read the private file")
        .await
        .unwrap();
    let card = collaboration_card(&room, "ada").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    let whole = if cfg!(target_os = "macos") {
        "your whole Mac"
    } else {
        "the whole machine"
    };
    assert_eq!(
        card["title"],
        format!(
            "Ada wants Bob to do work\n\nBob has more access than Ada. If you allow this, Bob does the work with its own access: {whole}."
        )
    );
    assert!(!card["title"].as_str().unwrap().contains("reach"));
    let names: Vec<&str> = card["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|option| option["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Allow", "Always", "Deny"]);
    assert_eq!(thread_of(&room, "ada~bob"), [] as [Value; 0]);
    assert!(room.session("bob").is_err());

    room.answer_permission("ada", &request_id, DENY)
        .await
        .unwrap();
    let refused = answer(&room, "ada", &id).await;
    assert_eq!(refused["cause"]["status"], "failed");
    assert!(
        refused["text"].as_str().unwrap().contains("denied"),
        "{refused}"
    );
    assert_eq!(thread_of(&room, "ada~bob"), [] as [Value; 0]);
    assert!(room.session("bob").is_err(), "nothing was handed to Bob");
    let settled = room
        .tape("ada")
        .into_iter()
        .find(|event| event["requestId"] == request_id)
        .unwrap();
    assert_eq!(settled["decision"], DENY);
}

#[tokio::test]
async fn explicit_whole_machine_hotline_agent_can_collaborate_without_a_card() {
    let room = room(
        "machine-collaboration",
        Fake::new(Scripted::new(answers("a1", "ready"))),
    );

    let id = ask(&room, "ada", "bob", "check the harbour").await.unwrap();

    assert_eq!(answer(&room, "ada", &id).await["text"], "check the harbour");
    assert!(room.tape("ada").into_iter().all(|event| {
        event["requestId"]
            .as_str()
            .is_none_or(|request_id| !request_id.starts_with(COLLAB_REQUEST_PREFIX))
    }));
    assert!(lock(&room.peers.waiting).is_empty());
}

#[tokio::test]
async fn collaboration_rechecks_reach_after_discovery() {
    let room = room(
        "collaboration-stale-reach",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let previous_caller = room.persona("ada").unwrap();
    let mut current_caller = previous_caller.clone();
    current_caller.reach = None;
    room.stop("ada").unwrap();
    enrol(room.log(), &current_caller);

    // Discovery saw Whole machine, but the delivery captured its lease after
    // the operator replaced that policy with workspace reach.
    let authorization = {
        let room = room.clone();
        tokio::spawn(async move {
            let target = room.persona("bob").unwrap();
            room.authorize_collaboration(
                &previous_caller,
                &target,
                &room.capability_lease("ada"),
                &room.capability_lease("bob"),
                false,
                None,
            )
            .await
        })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();
    assert!(authorization.await.unwrap().is_err());
    assert_eq!(thread_of(&room, "ada~bob"), [] as [Value; 0]);
}

/// Between equals, or down in access, no card asks: the receiver can do
/// nothing the sender could not.
#[tokio::test]
async fn between_equals_or_down_in_access_nothing_asks() {
    use crate::contract::Reach;
    for (name, ada, bob, from, to) in [
        ("equal-folders", None, None, "ada", "bob"),
        (
            "equal-machine",
            Some(Reach::Machine),
            Some(Reach::Machine),
            "ada",
            "bob",
        ),
        ("down", None, Some(Reach::Machine), "bob", "ada"),
    ] {
        let room = reach_room(
            &format!("no-card-{name}"),
            Fake::new(Scripted::new(answers("a1", "done"))),
            ada,
            bob,
        );
        let id = ask(&room, from, to, "check the harbour").await.unwrap();
        assert_eq!(
            answer(&room, from, &id).await["cause"]["status"],
            "done",
            "{name}"
        );
        assert_eq!(permission_cards(&room, from), 0, "{name}");
        assert!(lock(&room.peers.waiting).is_empty(), "{name}");
    }
}

/// Allow lets this one request through; the next one up in access asks
/// again, and Allow answers it again.
#[tokio::test]
async fn allow_lets_one_request_through_and_the_next_asks_again() {
    let room = up_room(
        "allow-once",
        Fake::new(Scripted::new(answers("a1", "done"))),
    );
    for (round, title) in ["one", "two"].into_iter().enumerate() {
        let id = ask(&room, "ada", "bob", title).await.unwrap();
        let card = collaboration_card(&room, "ada").await;
        room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW)
            .await
            .unwrap();
        assert_eq!(answer(&room, "ada", &id).await["cause"]["status"], "done");
        assert_eq!(permission_cards(&room, "ada"), round + 1);
    }
    let settled = room
        .tape("ada")
        .into_iter()
        .rfind(|event| kind_of(event) == "permission")
        .unwrap();
    assert_eq!(settled["decidedOptionName"], "Allow");
    assert_eq!(
        room.persona("bob").unwrap().allowed_senders,
        [] as [String; 0],
        "Allow keeps nothing"
    );
}

/// The person's Interrupt of either teammate's main-conversation turn
/// redirects that turn and decides nothing about a colleague: a
/// first-contact card up meanwhile stays up, and the person's answer to it
/// still lets the request through. Their Stop expires it.
#[tokio::test]
async fn an_interrupt_of_either_main_conversation_leaves_a_first_contact_card_up() {
    for whose in ["ada", "bob"] {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let room = up_room(
            &format!("interrupt-leaves-the-card-{whose}"),
            Fake::new(Scripted::new(answers("a1", "done")).gated(gate.clone())),
        );
        room.start(whose).await.unwrap();
        room.prompt(whose, "busy", None, None).await.unwrap();
        until("the person's turn is running", || room.mid_turn(whose)).await;
        let id = ask(&room, "ada", "bob", "one").await.unwrap();
        let card = collaboration_card(&room, "ada").await;
        room.interrupt(whose).unwrap();
        until("the cut turn is over", || !room.mid_turn(whose)).await;

        let still = room
            .tape("ada")
            .into_iter()
            .find(|event| event["requestId"] == card["requestId"])
            .unwrap();
        assert!(still.get("decision").is_none(), "{whose}: {still}");
        gate.add_permits(100);
        room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW)
            .await
            .unwrap();
        assert_eq!(
            answer(&room, "ada", &id).await["cause"]["status"],
            "done",
            "{whose}"
        );
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let room = up_room(
        "stop-expires-the-card",
        Fake::new(Scripted::new(answers("a1", "done")).gated(gate.clone())),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "busy", None, None).await.unwrap();
    until("the person's turn is running", || room.mid_turn("ada")).await;
    ask(&room, "ada", "bob", "one").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.cancel("ada").unwrap();
    until("the card has expired", || {
        room.tape("ada")
            .into_iter()
            .any(|event| event["requestId"] == card["requestId"] && event.get("decision").is_some())
    })
    .await;
    gate.add_permits(100);
}

#[tokio::test]
async fn permanent_consent_survives_peer_restart_and_uses_stable_sender_id() {
    let room = up_room(
        "workspace-permanent",
        Fake::new(Scripted::new(answers("a1", "done"))),
    );
    let first = ask(&room, "ada", "bob", "one").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_ALWAYS)
        .await
        .unwrap();
    assert_eq!(
        answer(&room, "ada", &first).await["cause"]["status"],
        "done"
    );
    assert_eq!(room.persona("bob").unwrap().allowed_senders, ["ada"]);

    room.stop("bob").unwrap();
    let mut renamed = room.persona("bob").unwrap();
    renamed.name = "Morgan".to_string();
    crate::room::append_persona(&room.log, &renamed).unwrap();
    let second = ask(&room, "ada", "Morgan", "two").await.unwrap();
    let answered = answer(&room, "ada", &second).await;
    assert_eq!(answered["cause"]["name"], "Morgan");
    assert_eq!(answered["cause"]["status"], "done");
    assert!(room.persona("morgan").is_err());
    assert_eq!(room.persona("bob").unwrap().allowed_senders, ["ada"]);
    let third = ask(&room, "ada", "bob", "three").await.unwrap();
    assert_eq!(
        answer(&room, "ada", &third).await["cause"]["status"],
        "done"
    );
    assert_eq!(permission_cards(&room, "ada"), 1);
}

#[tokio::test]
async fn removing_a_permanent_grant_revokes_cached_work_and_requires_consent_again() {
    let room = up_room(
        "workspace-remove-grant",
        Fake::new(Scripted::new(answers("a1", "done"))),
    );
    let first = ask(&room, "ada", "bob", "one").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_ALWAYS)
        .await
        .unwrap();
    answer(&room, "ada", &first).await;
    // Bob's main conversation was woken for it under the grant: its tools
    // are the work the grant left running.
    until("bob is up", || room.session("bob").is_ok()).await;
    let answering = TeammateTools::new(&room, "bob")
        .with_capability(room.session("bob").unwrap().capability.clone());
    assert!(answering.call("list_teammates", &json!({})).await.is_ok());

    // This is the ordering used by persona.update: revoke execution before
    // writing the replacement record, then reactivate the new epoch. The old
    // tools must remain dead after removal.
    room.invalidate("bob").unwrap();
    let mut bob = room.persona("bob").unwrap();
    bob.allowed_senders.clear();
    crate::room::append_persona(&room.log, &bob).unwrap();
    room.reattach("bob").await.unwrap();
    assert!(answering.call("list_teammates", &json!({})).await.is_err());

    let after_removal = ask(&room, "ada", "bob", "two").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();
    assert_eq!(
        answer(&room, "ada", &after_removal).await["cause"]["status"],
        "failed"
    );
}

/// A turn that thinks once and then answers.
fn answers(id: &str, text: &str) -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Thought,
            id: format!("{id}-thought"),
            text: "let me look".to_string(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: id.to_string(),
            text: text.to_string(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]
}

fn thread_of(room: &Room, key: &str) -> Vec<Value> {
    room.log.load(&StreamId::Pair(key.to_string()))
}

/// Waits for the `count`th delivery on a teammate's tape to have been heard
/// and its turn to have ended, so a test never races the exchange's task or
/// the turn the answer started.
async fn delivered(room: &Room, persona_id: &str, count: usize) -> Value {
    for _ in 0..300 {
        let tape = room.tape(persona_id);
        let deliveries: Vec<&Value> = tape
            .iter()
            .filter(|event| kind_of(event) == "delivery")
            .collect();
        if let Some(delivery) = deliveries.get(count - 1)
            && delivery["receipt"] == "read"
            && tape
                .iter()
                .skip_while(|event| event["id"] != delivery["id"])
                .any(|event| kind_of(event) == "turn")
        {
            return (*delivery).clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("{persona_id} was never handed delivery {count}")
}

/// A delivery says where it came from in a field, and keeps it as it is read:
/// the turn that takes it has no id to parse. One written before the field
/// still reads, as without a source.
#[tokio::test]
async fn a_delivery_carries_where_it_came_from_and_keeps_it_when_read() {
    let agents = Fake::new(Scripted::new(answers("a1", "ok")));
    let room = room("provenance", agents);
    room.start("ada").await.unwrap();
    room.deliver_identified(
        "ada",
        &new_id(),
        DeliveryCause::Peer {
            request_id: Some("req-1".to_string()),
            persona_id: "bob".to_string(),
            name: "Bob".to_string(),
            thread_key: "ada~bob".to_string(),
            status: PeerStatus::Done,
            about: "what broke?".to_string(),
            answer: None,
        },
        "the winch".to_string(),
    )
    .await
    .unwrap();
    room.deliver_identified(
        "ada",
        &new_id(),
        DeliveryCause::Answer {
            action_id: "act-1".to_string(),
            status: HumanActionStatus::Done,
            about: "which key?".to_string(),
        },
        "the blue one".to_string(),
    )
    .await
    .unwrap();

    let delivered = || {
        room.tape("ada")
            .into_iter()
            .filter(|event| kind_of(event) == "delivery")
            .collect::<Vec<_>>()
    };
    for _ in 0..500 {
        if delivered().iter().all(|event| event["receipt"] == "read") && delivered().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let delivered = delivered();
    assert_eq!(delivered.len(), 2);
    assert_eq!(
        delivered[0]["from"],
        serde_json::json!({"thread": "ada~bob", "kind": "pair", "request": "req-1"})
    );
    assert_eq!(
        delivered[1]["from"],
        serde_json::json!({"thread": "ada", "kind": "dm", "request": "act-1"})
    );
    assert!(delivered.iter().all(|event| event["receipt"] == "read"));
    let from: DeliveryFrom = serde_json::from_value(delivered[0]["from"].clone()).unwrap();
    assert_eq!(from.thread(), ThreadId::pair("ada~bob"));

    let before: TranscriptEvent = serde_json::from_value(serde_json::json!({
        "kind": "delivery", "id": "d0", "ts": 1, "text": "old",
        "cause": {"kind": "peer", "personaId": "bob", "name": "Bob",
                  "threadKey": "ada~bob", "status": "done", "about": "x"}
    }))
    .unwrap();
    assert!(matches!(
        before,
        TranscriptEvent::Delivery { from: None, .. }
    ));
}

/// What a restart left: a delivery the agent never read is handed to it
/// again, once, and one it has read is not.
#[tokio::test]
async fn a_restart_hands_on_an_unheard_delivery_once() {
    let agents = Fake::new(Scripted::new(answers("a1", "ok")));
    let room = room("recover", agents.clone());
    room.start("ada").await.unwrap();
    let now = now_ms();
    let cause = DeliveryCause::Peer {
        request_id: None,
        persona_id: "bob".to_string(),
        name: "Bob".to_string(),
        thread_key: "ada~bob".to_string(),
        status: PeerStatus::Done,
        about: "what broke?".to_string(),
        answer: None,
    };
    room.write(
        "ada",
        &TranscriptEvent::Delivery {
            id: "d-unheard".to_string(),
            ts: now,
            cause: cause.clone(),
            text: "the winch".to_string(),
            receipt: Some(Receipt::Sent),
            from: None,
        },
    );
    room.write(
        "ada",
        &TranscriptEvent::Delivery {
            id: "d-heard".to_string(),
            ts: now,
            cause,
            text: "already heard".to_string(),
            receipt: Some(Receipt::Read),
            from: None,
        },
    );

    room.recover_exchanges().await;
    room.recover_exchanges().await;

    let heard = delivered(&room, "ada", 1).await;
    assert_eq!(heard["id"], "d-unheard");
    let prompts = agents.prompts();
    assert_eq!(
        prompts
            .iter()
            .filter(|line| line.contains("the winch"))
            .count(),
        1
    );
    assert!(prompts.iter().all(|line| !line.contains("already heard")));
}

#[tokio::test]
async fn a_teammate_this_room_does_not_have_is_refused_in_a_sentence() {
    let room = room("stranger", Fake::new(Scripted::new(answers("a1", "aye"))));
    let refused = ask(&room, "ada", "Cal", "hello?").await.unwrap_err();
    assert!(refused.contains("Cal"), "{refused}");
    assert!(refused.contains("list_teammates"), "{refused}");
    assert_eq!(thread_of(&room, "ada~cal"), [] as [Value; 0]);

    let itself = ask(&room, "ada", "Ada", "hello?").await.unwrap_err();
    assert!(itself.contains("not to yourself"), "{itself}");
}

/// A card nobody answered is dropped with its ask: it is expired on the tape
/// and cannot be answered later, so it grants nothing.
#[tokio::test]
async fn a_dropped_collaboration_wait_is_expired_and_cannot_be_answered_later() {
    let room = up_room(
        "dropped-collaboration-wait",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    ask(&room, "ada", "bob", "wait for approval").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    room.stop_exchange("ada", "bob").unwrap();
    until("the wait is dropped", || {
        lock(&room.peers.waiting).is_empty()
    })
    .await;
    assert!(
        room.answer_permission("ada", &request_id, ALLOW_ALWAYS)
            .await
            .unwrap_err()
            .contains("no longer waiting")
    );
    let settled = room
        .tape("ada")
        .into_iter()
        .find(|event| event["requestId"] == request_id)
        .expect("the dropped wait leaves an expired collaboration card");
    assert_eq!(settled["decision"], "expired");
    assert_eq!(
        room.persona("bob").unwrap().allowed_senders,
        [] as [String; 0]
    );
    assert!(
        crate::store::inbox::received(&room.log, "bob").is_empty(),
        "nothing was handed to Bob"
    );
}

#[tokio::test]
async fn an_answered_collaboration_cannot_start_after_a_chapter_generation_changes() {
    let room = up_room(
        "collaboration-generation-race",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let id = ask(&room, "ada", "bob", "start after approval")
        .await
        .unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW)
        .await
        .unwrap();
    room.peers.advance_generation("ada");

    let refused = answer(&room, "ada", &id).await;
    assert_eq!(refused["cause"]["status"], "failed");
    assert!(
        refused["text"]
            .as_str()
            .unwrap()
            .contains("approval expired"),
        "{refused}"
    );
    assert_eq!(thread_of(&room, "ada~bob"), [] as [Value; 0]);
    assert_eq!(
        crate::store::inbox::received(&room.log, "bob"),
        [] as [crate::contract::InboxItem; 0]
    );
}

/// The receipts a reader moves in a pair's thread.
///
/// Every one of these is about the *kind* of event and never its text: a
/// tick the model could produce by writing the right sentence would be a
/// lie the reader has no way to check.
mod receipts {
    use super::*;

    fn user(id: &str, text: &str) -> TranscriptEvent {
        TranscriptEvent::User {
            handoff_thread: None,
            id: id.to_string(),
            ts: 1,
            text: text.to_string(),
            attachments: None,
            reactions: None,
            reply_to: None,
            scheduled: None,
            ring: None,
            receipt: None,
            client: None,
        }
    }

    fn agent(id: &str, text: &str) -> TranscriptEvent {
        TranscriptEvent::Agent {
            handoff_thread: None,
            id: id.to_string(),
            ts: 2,
            text: text.to_string(),
            attachments: None,
            reactions: None,
            ring: None,
            receipt: None,
            spoken: None,
            silent: None,
            from: None,
            scheduled: None,
            from_item: None,
        }
    }

    fn thought(text: &str) -> TranscriptEvent {
        TranscriptEvent::Thought {
            id: "th".to_string(),
            ts: 3,
            text: text.to_string(),
        }
    }

    fn id_of(event: &TranscriptEvent) -> String {
        serde_json::to_value(event).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn receipt(event: &TranscriptEvent) -> Option<Receipt> {
        receipt_of(event).flatten()
    }

    #[test]
    fn the_ladder_only_climbs() {
        assert_eq!(higher(None, Receipt::Sent), Receipt::Sent);
        assert_eq!(higher(Some(Receipt::Sent), Receipt::Read), Receipt::Read);
        assert_eq!(higher(Some(Receipt::Read), Receipt::Sent), Receipt::Read);
        assert_eq!(higher(Some(Receipt::Read), Receipt::Read), Receipt::Read);
        // A message that already carries a receipt is not lowered by a stamp.
        let already = stamped(user("u1", ""), Receipt::Read);
        assert_eq!(
            receipt(&stamped(already, Receipt::Sent)),
            Some(Receipt::Read)
        );
    }

    fn stored(events: &[TranscriptEvent]) -> Vec<Value> {
        events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect()
    }

    #[test]
    fn a_replys_read_receipt_names_the_messages_it_moves() {
        let one = stamped(agent("a1", ""), Receipt::Sent);
        let two = stamped(agent("a2", ""), Receipt::Sent);
        let events = stored(&[user("u1", ""), one, two]);
        let moved = read_receipt_updates(&events, &["a1".to_string(), "a2".to_string()]);
        assert_eq!(moved.iter().map(id_of).collect::<Vec<_>>(), ["a1", "a2"]);
        assert!(
            moved
                .iter()
                .all(|event| receipt(event) == Some(Receipt::Read))
        );
    }

    #[test]
    fn a_receipt_naming_nothing_or_something_already_read_writes_nothing() {
        let said = stored(&[stamped(agent("a1", ""), Receipt::Read)]);
        assert_eq!(
            read_receipt_updates(&said, &["a1".to_string()]),
            [] as [TranscriptEvent; 0]
        );
        assert_eq!(
            read_receipt_updates(&said, &["no-such-id".to_string()]),
            [] as [TranscriptEvent; 0]
        );
        assert_eq!(
            read_receipt_updates(&[], &["anything".to_string()]),
            [] as [TranscriptEvent; 0]
        );
    }

    #[test]
    fn a_receipt_cannot_move_machinery() {
        let machinery = stored(&[thought("")]);
        assert_eq!(
            read_receipt_updates(&machinery, &["th".to_string()]),
            [] as [TranscriptEvent; 0]
        );
    }
}

/// Always covers later items, whatever they want, even after another
/// operator turn.
#[tokio::test]
async fn collaboration_consent_covers_separate_operator_turns_and_both_intents() {
    {
        let decision = ALLOW_ALWAYS;
        let room = up_room(
            &format!("consent-across-turns-{decision}"),
            Fake::new(Scripted::turns(
                (0..30)
                    .map(|i| answers(&format!("answer-{i}"), "done"))
                    .collect(),
            )),
        );
        room.start("ada").await.unwrap();
        for (index, wants) in ["answer", "work", "answer"].iter().enumerate() {
            room.prompt("ada", "Please ask Bob for another check", None, None)
                .await
                .unwrap();
            let id = send_wanting(&room, "ada", "bob", &format!("check {index}"), wants)
                .await
                .unwrap();
            if index == 0 {
                let card = collaboration_card(&room, "ada").await;
                room.answer_permission("ada", card["requestId"].as_str().unwrap(), decision)
                    .await
                    .unwrap();
            }
            let landed = answer(&room, "ada", &id).await;
            assert_eq!(landed["cause"]["status"], "done", "{decision} {wants}");
            assert!(
                room.tape("ada").iter().all(|event| {
                    event["kind"] != "permission" || event.get("decision").is_some()
                }),
                "{decision} requested approval again for {wants}"
            );
        }
        assert_eq!(
            room.tape("ada")
                .iter()
                .filter(|event| event["kind"] == "permission")
                .count(),
            1
        );
    }
}
