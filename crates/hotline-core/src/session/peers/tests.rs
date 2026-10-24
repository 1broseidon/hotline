//! Two teammates talking, driven by the scripted driver.
//!
//! Nothing here reaches a model. What is under test is what an ask needs and
//! leaves behind: the person's grant, the thread both sides share, the
//! receipts on it, and what each side hears.

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

/// A pair in the ordinary workspace reach, so an ask needs the person's
/// grant.
fn workspace_room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    let mut ada = persona("ada");
    ada.reach = None;
    enrol(&log, &ada);
    let mut bob = persona("bob");
    bob.name = "Bob".to_string();
    bob.reach = None;
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

/// `from` asks `to` with its tools, as an agent does, and the request's id.
async fn ask(room: &Arc<Room>, from: &str, to: &str, message: &str) -> Result<String, String> {
    let sent = TeammateTools::new(room, from)
        .call("message_teammate", &json!({ "to": to, "message": message }))
        .await?;
    Ok(serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string())
}

/// What came back to `whose` for this request, once it has.
async fn answer(room: &Room, whose: &str, id: &str) -> Value {
    let found = || {
        room.tape(whose)
            .into_iter()
            .find(|event| kind_of(event) == "delivery" && event["cause"]["requestId"] == id)
    };
    until("the answer is delivered", || found().is_some()).await;
    found().unwrap()
}

#[tokio::test]
async fn workspace_delivery_requires_consent_before_peer_side_effects() {
    let room = workspace_room(
        "workspace-deny",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let id = ask(&room, "ada", "bob", "read the private file")
        .await
        .unwrap();
    let card = collaboration_card(&room, "ada").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    assert_eq!(
        card["title"],
        "Allow Ada to ask Bob to work?\n\nAda's questions reach Bob's main conversation with you, and Bob answers from everything said there, including what you told it. Bob can also take handoffs in a work thread of its own, and uses its own workspace and enabled tools to fulfill Ada's requests."
    );
    assert_eq!(card["options"][0]["name"], "Allow this session");
    assert_eq!(card["options"][1]["name"], "Always allow Ada");
    assert_eq!(card["options"][2]["name"], "Deny");
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

    assert_eq!(answer(&room, "ada", &id).await["text"], "ready");
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

#[tokio::test]
async fn session_consent_is_directional_and_expires_when_a_side_stops() {
    let room = workspace_room(
        "workspace-session",
        Fake::new(Scripted::new(answers("a1", "done"))),
    );
    let first = ask(&room, "ada", "bob", "one").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    assert_eq!(
        answer(&room, "ada", &first).await["cause"]["status"],
        "done"
    );

    // The same direction is covered without a new card, and the reverse
    // direction has no grant of its own.
    let second = ask(&room, "ada", "bob", "two").await.unwrap();
    assert_eq!(
        answer(&room, "ada", &second).await["cause"]["status"],
        "done"
    );
    assert_eq!(permission_cards(&room, "ada"), 1);
    let reverse = ask(&room, "bob", "ada", "back").await.unwrap();
    let reverse_card = collaboration_card(&room, "bob").await;
    room.answer_permission("bob", reverse_card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();
    assert_eq!(
        answer(&room, "bob", &reverse).await["cause"]["status"],
        "failed"
    );

    // Stopping either participant ends the temporary grant; a later request
    // must ask again.
    room.stop("bob").unwrap();
    let after_stop = ask(&room, "ada", "bob", "three").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    assert_eq!(
        answer(&room, "ada", &after_stop).await["cause"]["status"],
        "done"
    );
    assert_eq!(permission_cards(&room, "ada"), 2);
}

/// The person's Interrupt of either teammate's main-conversation turn
/// redirects that turn and decides nothing about the colleague: a
/// first-contact card up meanwhile stays up, and the person's answer to it
/// still lets the request through.
#[tokio::test]
async fn an_interrupt_of_either_main_conversation_leaves_a_first_contact_card_up() {
    for whose in ["ada", "bob"] {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let room = workspace_room(
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
        room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
            .await
            .unwrap();
        assert_eq!(
            answer(&room, "ada", &id).await["cause"]["status"],
            "done",
            "{whose}"
        );
    }
}

#[tokio::test]
async fn permanent_consent_survives_peer_restart_and_uses_stable_sender_id() {
    let room = workspace_room(
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
    let room = workspace_room(
        "workspace-remove-grant",
        Fake::new(Scripted::new(answers("a1", "done"))),
    );
    let first = ask(&room, "ada", "bob", "one").await.unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_ALWAYS)
        .await
        .unwrap();
    answer(&room, "ada", &first).await;
    // Bob's main conversation answered under the grant: its tools are the
    // work the grant left running.
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

fn kinds(events: &[Value]) -> Vec<&str> {
    events.iter().map(kind_of).collect()
}

/// The thread's key decides which side is stored as `user`, so the same pair
/// reads the same way up whichever of them asked.
#[tokio::test]
async fn the_thread_is_stored_the_same_way_up_whoever_asked() {
    let room = room("orient", Fake::new(Scripted::new(answers("a1", "on it"))));
    let id = ask(&room, "bob", "ada", "can you look?").await.unwrap();
    answer(&room, "bob", &id).await;

    let events = thread_of(&room, "ada~bob");
    assert_eq!(
        kinds(&events),
        ["agent", "user"],
        "ada is the key's first participant, so ada's words are the user side"
    );
    assert_eq!(events[0]["text"], "can you look?");
    assert_eq!(events[1]["text"], "on it");
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

/// A teammate's own session sends and moves on: the tool says the message
/// went, and the answer comes back into the sender's conversation as a
/// delivery the agent is handed on a turn of its own.
#[tokio::test]
async fn the_tool_returns_once_sent_and_the_answer_is_delivered() {
    let agents = Fake::new(Scripted::turns(vec![
        answers("a1", "the winch"),
        answers("a2", "thanks, fixing it"),
    ]));
    let room = room("tool", agents.clone());
    let sent = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &serde_json::json!({ "to": "Bob", "message": "what broke?\nthe crane stopped" }),
        )
        .await
        .unwrap();

    let sent: Value = serde_json::from_str(&sent).unwrap();
    assert_eq!(sent["sent"], true);
    assert_eq!(sent["to"], "Bob");
    assert!(
        sent.get("reply").is_none(),
        "the answer is not in the result"
    );

    let delivery = delivered(&room, "ada", 1).await;
    assert_eq!(delivery["text"], "the winch");
    assert_eq!(delivery["cause"]["kind"], "peer");
    assert_eq!(delivery["cause"]["personaId"], "bob");
    assert_eq!(delivery["cause"]["name"], "Bob");
    assert_eq!(delivery["cause"]["threadKey"], "ada~bob");
    assert_eq!(delivery["cause"]["status"], "done");
    assert_eq!(delivery["cause"]["about"], "what broke?");
    let heard = agents.prompts();
    let last = heard.last().unwrap();
    assert!(
        last.contains("Bob answered the message you sent them (\"what broke?\")")
            && last.contains("the winch"),
        "{last}"
    );
    assert!(
        room.tape("ada")
            .iter()
            .all(|event| kind_of(event) != "user"),
        "a delivery is not a message from the person"
    );
}

/// An exchange that ends without an answer is delivered too, so the sender
/// is never left expecting one.
#[tokio::test]
async fn a_refused_exchange_is_delivered_as_unanswered() {
    let room = workspace_room(
        "tool-denied",
        Fake::new(Scripted::new(answers("a1", "noted"))),
    );
    let sent = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({ "to": "bob", "message": "can I use your build?" }),
        )
        .await
        .unwrap();
    assert!(sent.contains("\"sent\":true"), "{sent}");
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();

    let delivery = delivered(&room, "ada", 1).await;
    assert_eq!(delivery["cause"]["status"], "failed");
    assert_eq!(delivery["cause"]["about"], "can I use your build?");
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
        },
        "the winch".to_string(),
        None,
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
        None,
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
    assert!(itself.contains("cannot message itself"), "{itself}");
}

#[tokio::test]
async fn a_thread_is_listed_for_both_sides_and_its_reply_can_be_read() {
    let room = room("listed", Fake::new(Scripted::new(answers("a1", "aye"))));
    let id = ask(&room, "ada", "bob", "are you free?").await.unwrap();
    answer(&room, "ada", &id).await;

    let listed = room.peer_threads("ada");
    assert_eq!(listed.len(), 1);
    let summary = &listed[0];
    assert_eq!(summary.thread_key, "ada~bob");
    assert_eq!(summary.with_persona_id, "bob");
    assert_eq!(summary.with_name, "Bob");
    assert!(!summary.waiting);
    assert_eq!(summary.working_persona_id, None);
    let preview = summary.preview.as_ref().expect("the last thing said");
    assert_eq!(preview.from_name, "Bob");
    assert_eq!(preview.text, "aye");
    // Bob's side of the same conversation names Ada.
    assert_eq!(room.peer_threads("bob")[0].with_name, "Ada");

    let reply_id = thread_of(&room, "ada~bob")
        .into_iter()
        .find(|event| event["text"] == "aye")
        .map(|event| event["id"].as_str().unwrap().to_string())
        .unwrap();
    assert_eq!(
        room.mark_peer_read("ada~bob", std::slice::from_ref(&reply_id)),
        1
    );
    // A receipt that has already landed moves nothing the second time.
    assert_eq!(
        room.mark_peer_read("ada~bob", std::slice::from_ref(&reply_id)),
        0
    );
    let read = thread_of(&room, "ada~bob")
        .into_iter()
        .find(|event| event["id"] == reply_id.as_str())
        .unwrap();
    assert_eq!(read["receipt"], "read");
}

/// A card nobody answered is dropped with its ask: it is expired on the tape
/// and cannot be answered later, so it grants nothing.
#[tokio::test]
async fn a_dropped_collaboration_wait_is_expired_and_cannot_be_answered_later() {
    let room = workspace_room(
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
    assert!(room.session("bob").is_err(), "nothing was handed to Bob");
}

#[tokio::test]
async fn an_answered_collaboration_cannot_start_after_a_chapter_generation_changes() {
    let room = workspace_room(
        "collaboration-generation-race",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let id = ask(&room, "ada", "bob", "start after approval")
        .await
        .unwrap();
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
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
    assert!(room.session("bob").is_err());
    assert!(lock(&room.peers.session_grants).is_empty());
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

/// An approval covers later asks and handoffs, even after another operator
/// turn.
#[tokio::test]
async fn collaboration_consent_covers_separate_operator_turns_and_both_intents() {
    for decision in [ALLOW_SESSION, ALLOW_ALWAYS] {
        let room = workspace_room(
            &format!("consent-across-turns-{decision}"),
            Fake::new(Scripted::turns(
                (0..30)
                    .map(|i| answers(&format!("answer-{i}"), "done"))
                    .collect(),
            )),
        );
        room.start("ada").await.unwrap();
        for (index, intent) in ["ask", "handoff", "ask"].iter().enumerate() {
            room.prompt("ada", "Please ask Bob for another check", None, None)
                .await
                .unwrap();
            let result = TeammateTools::new(&room, "ada")
                .call(
                    "message_teammate",
                    &json!({
                        "to": "bob", "message": format!("check {index}"), "intent": intent
                    }),
                )
                .await
                .unwrap();
            let result: Value = serde_json::from_str(&result).unwrap();
            let request_id = result["requestId"].as_str().unwrap();
            if index == 0 {
                let card = collaboration_card(&room, "ada").await;
                room.answer_permission("ada", card["requestId"].as_str().unwrap(), decision)
                    .await
                    .unwrap();
            }
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                loop {
                    if let Some(result) = room.tape("ada").iter().find(|event| {
                        event["kind"] == "delivery" && event["cause"]["requestId"] == request_id
                    }) {
                        assert_eq!(result["cause"]["status"], "done", "{result}");
                        break;
                    }
                    assert!(
                        room.tape("ada").iter().all(|event| {
                            event["kind"] != "permission" || event.get("decision").is_some()
                        }),
                        "{decision} requested approval again for {intent}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the approved request should return a result");
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

/// What two teammates said to each other is found by either of them, and the
/// hit names the thread; a rebuild keeps it.
#[tokio::test]
async fn a_pair_thread_is_searchable_by_both_teammates_and_survives_a_rebuild() {
    let room = room(
        "peer-search",
        Fake::new(Scripted::new(answers("a1", "the winch is jammed"))),
    );
    let id = ask(&room, "ada", "Bob", "what about the capstan?")
        .await
        .unwrap();
    answer(&room, "ada", &id).await;
    for (who, word) in [("ada", "capstan"), ("bob", "capstan"), ("ada", "winch")] {
        let found =
            crate::store::search::search_teammate(room.log.root(), who, word, None).unwrap();
        let text = found.to_string();
        assert!(text.contains("pair:ada~bob"), "{who} {word}: {text}");
    }
    for who in ["ada", "bob"] {
        lock(&room.indexer).as_mut().unwrap().reindex(who).unwrap();
        let found =
            crate::store::search::search_teammate(room.log.root(), who, "capstan", None).unwrap();
        assert!(found.to_string().contains("pair:ada~bob"), "{who}: {found}");
    }
}
