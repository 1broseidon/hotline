//! Two teammates talking, driven by the scripted driver.
//!
//! Nothing here reaches a model. What is under test is what a delivery leaves
//! behind: the thread both sides share, the receipts on it, and the marker on
//! each of their own tapes.

use super::*;
use crate::contract::{ChapterClose, NoticeLevel, ToolStatus};
use crate::driver::rig::Said;
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
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

/// A pair in the ordinary workspace reach. The tests below deliberately keep
/// both teammates out of a main session: a peer request itself must carry the
/// caller and recipient leases that make the approval temporary.
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
    for _ in 0..100 {
        if let Some(card) = room.tape(persona_id).into_iter().find(|event| {
            kind_of(event) == "permission"
                && event["requestId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(COLLAB_REQUEST_PREFIX))
                && event.get("decision").is_none()
        }) {
            return card;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("the collaboration card did not reach the tape")
}

#[tokio::test]
async fn workspace_delivery_requires_consent_before_peer_side_effects() {
    let room = workspace_room(
        "workspace-deny",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "read the private file").await })
    };
    let card = collaboration_card(&room, "ada").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    assert_eq!(
        card["title"],
        "Allow Ada to ask Bob to work?\n\nBob can receive handoffs into its main conversation, use its own context, workspace and enabled tools to fulfill Ada's requests and return results."
    );
    assert_eq!(card["options"][0]["name"], "Allow this session");
    assert_eq!(card["options"][1]["name"], "Always allow Ada");
    assert_eq!(card["options"][2]["name"], "Deny");
    assert!(thread_of(&room, "ada~bob").is_empty());
    assert!(lock(&room.peers.sessions).is_empty());

    room.answer_permission("ada", &request_id, DENY)
        .await
        .unwrap();
    let error = delivery.await.unwrap().unwrap_err();
    assert!(error.contains("denied"), "{error}");
    assert!(thread_of(&room, "ada~bob").is_empty());
    assert!(lock(&room.peers.sessions).is_empty());
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

    let answered = room
        .deliver("ada", "bob", "check the harbour")
        .await
        .unwrap();

    assert_eq!(answered.reply, "ready");
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
            )
            .await
        })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();
    assert!(authorization.await.unwrap().is_err());
    assert!(thread_of(&room, "ada~bob").is_empty());
}

#[tokio::test]
async fn session_consent_is_directional_and_expires_when_a_side_stops() {
    let room = workspace_room(
        "workspace-session",
        Fake::new(Scripted::turns(vec![
            answers("a1", "first"),
            answers("a2", "second"),
            answers("a3", "after restart"),
        ])),
    );
    let first = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "one").await })
    };
    let card = collaboration_card(&room, "ada").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    room.answer_permission("ada", &request_id, ALLOW_SESSION)
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().unwrap().reply, "first");

    // The same live caller/recipient peer session is covered without a new
    // prompt, and the reverse direction has no grant of its own.
    assert_eq!(
        room.deliver("ada", "bob", "two").await.unwrap().reply,
        "second"
    );
    let reverse = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("bob", "ada", "back").await })
    };
    let reverse_card = collaboration_card(&room, "bob").await;
    room.answer_permission("bob", reverse_card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();
    assert!(reverse.await.unwrap().is_err());

    // Stopping either participant revokes the peer leases and drops the
    // temporary grant; a later request must ask again.
    room.stop("bob").unwrap();
    let after_stop = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "three").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    assert_eq!(after_stop.await.unwrap().unwrap().reply, "after restart");
}

#[tokio::test]
async fn permanent_consent_survives_peer_restart_and_uses_stable_sender_id() {
    let room = workspace_room(
        "workspace-permanent",
        Fake::new(Scripted::turns(vec![
            answers("a1", "first"),
            answers("a2", "second"),
            answers("a3", "after rename"),
        ])),
    );
    let first = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "one").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_ALWAYS)
        .await
        .unwrap();
    assert_eq!(first.await.unwrap().unwrap().reply, "first");
    assert_eq!(room.persona("bob").unwrap().allowed_senders, ["ada"]);

    room.stop("bob").unwrap();
    let mut renamed = room.persona("bob").unwrap();
    renamed.name = "Morgan".to_string();
    crate::room::append_persona(&room.log, &renamed).unwrap();
    let second = room.deliver("ada", "Morgan", "two").await.unwrap();
    assert_eq!(second.from, "Morgan");
    assert_eq!(second.reply, "second");
    assert!(room.persona("morgan").is_err());
    assert_eq!(room.persona("bob").unwrap().allowed_senders, ["ada"]);
    assert_eq!(
        room.deliver("ada", "bob", "three").await.unwrap().reply,
        "after rename"
    );
}

#[tokio::test]
async fn removing_a_permanent_grant_revokes_cached_work_and_requires_consent_again() {
    let room = workspace_room(
        "workspace-remove-grant",
        Fake::new(Scripted::new(answers("a1", "first"))),
    );
    let first = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "one").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_ALWAYS)
        .await
        .unwrap();
    first.await.unwrap().unwrap();

    let cached = lock(&room.peers.sessions)
        .get(&(String::from("ada"), String::from("bob")))
        .cloned()
        .expect("the standing grant left a cached peer session");
    let cached_tools =
        TeammateTools::new(&room, "bob").with_capability(cached.target_capability.clone());

    // This is the ordering used by persona.update: revoke execution before
    // writing the replacement record, then reactivate the new epoch. The old
    // target tools and cached peer session must remain dead after removal.
    room.invalidate("bob").unwrap();
    let mut bob = room.persona("bob").unwrap();
    bob.allowed_senders.clear();
    crate::room::append_persona(&room.log, &bob).unwrap();
    room.reattach("bob").await.unwrap();
    assert!(
        cached_tools
            .call("list_teammates", &json!({}))
            .await
            .is_err()
    );
    assert!(lock(&room.peers.sessions).is_empty());

    let after_removal = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "two").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), DENY)
        .await
        .unwrap();
    assert!(after_removal.await.unwrap().is_err());
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
    room.log.load(&StreamId::Thread(key.to_string()))
}

fn kinds(events: &[Value]) -> Vec<&str> {
    events.iter().map(kind_of).collect()
}

/// The peer marker on a teammate's own tape, folded to one line.
fn marker_on(room: &Room, persona_id: &str) -> Value {
    room.tape(persona_id)
        .into_iter()
        .find(|event| kind_of(event) == "peer")
        .expect("the tape carries a peer marker")
}

#[tokio::test]
async fn a_delivery_writes_both_sides_of_the_thread_with_its_receipts() {
    let room = room(
        "deliver",
        Fake::new(Scripted::new(answers("a1", "it jammed on the second lift"))),
    );

    let answered = room
        .deliver("ada", "Bob", "did the crane jam?")
        .await
        .unwrap();
    assert_eq!(answered.from, "Bob");
    assert_eq!(answered.reply, "it jammed on the second lift");

    let events = thread_of(&room, "ada~bob");
    assert_eq!(
        kinds(&events),
        ["user", "thought", "agent", "turn"],
        "the caller's line is in the thread before the turn that answers it"
    );
    assert_eq!(events[0]["text"], "did the crane jam?");
    assert_eq!(
        events[0]["receipt"], "read",
        "the recipient's first sign of a turn is what reads the message"
    );
    assert_eq!(events[2]["text"], "it jammed on the second lift");
    assert_eq!(
        events[2]["receipt"], "sent",
        "whether the caller read the reply is not knowable here"
    );
    // The words of the exchange are the thread's; neither tape carries them.
    assert!(
        !room
            .tape("ada")
            .iter()
            .any(|event| kind_of(event) == "user")
    );
    assert!(
        !room
            .tape("bob")
            .iter()
            .any(|event| kind_of(event) == "agent")
    );
}

/// A peer reply is paced in the same funnel as a tape: two paragraphs are two
/// bubbles, and history reads them as the one thing the model said.
#[tokio::test]
async fn a_peer_reply_is_paced_the_same_way() {
    let first = "Paragraph 1 is long enough to stand as its own bubble in the chat.";
    let second = "Paragraph 2 is long enough to stand as its own bubble in the chat.";
    let text = format!("{first}\n\n{second}");
    let room = room("paced", Fake::new(Scripted::new(answers("a1", &text))));

    let answered = room.deliver("ada", "Bob", "status?").await.unwrap();
    assert_eq!(answered.reply, text);

    let events = thread_of(&room, "ada~bob");
    let agents: Vec<&Value> = events
        .iter()
        .filter(|event| kind_of(event) == "agent")
        .collect();
    assert_eq!(agents.len(), 2, "{}", kinds(&events).join(", "));
    assert_eq!(agents[0]["id"], "a1");
    assert_eq!(agents[1]["id"], "a1-2");
    assert_eq!(agents[0]["ts"], agents[1]["ts"]);
    assert_eq!(agents[0]["text"], first);
    assert_eq!(agents[1]["text"], second);
    assert_eq!(
        crate::session::tests::words(said_in(&events, false)),
        [Said::User("status?".to_string()), Said::Agent(text),]
    );
}

#[tokio::test]
async fn each_side_gets_a_marker_on_its_own_tape() {
    let room = room("markers", Fake::new(Scripted::new(answers("a1", "aye"))));
    room.deliver("ada", "bob", "are you free?").await.unwrap();

    let caller = marker_on(&room, "ada");
    assert_eq!(caller["threadKey"], "ada~bob");
    assert_eq!(caller["role"], "caller");
    assert_eq!(caller["withPersonaId"], "bob");
    assert_eq!(caller["withName"], "Bob");
    assert_eq!(caller["exchanges"], 1);
    assert_eq!(caller["status"], "done");

    let target = marker_on(&room, "bob");
    assert_eq!(target["role"], "target");
    assert_eq!(target["withPersonaId"], "ada");
    assert_eq!(target["withName"], "Ada");
    assert_eq!(target["exchanges"], 1);
    assert_eq!(target["status"], "done");
}

#[tokio::test]
async fn a_second_delivery_bumps_the_same_marker() {
    let room = room(
        "second",
        Fake::new(Scripted::turns(vec![
            answers("a1", "first"),
            answers("a2", "second"),
        ])),
    );
    room.deliver("ada", "bob", "one").await.unwrap();
    let first = marker_on(&room, "ada");
    let again = room.deliver("ada", "bob", "two").await.unwrap();
    assert_eq!(again.reply, "second");

    let second = marker_on(&room, "ada");
    assert_eq!(
        second["id"], first["id"],
        "a run of exchanges is one line, superseded by id"
    );
    assert_eq!(second["exchanges"], 2);
    assert_eq!(
        thread_of(&room, "ada~bob")
            .iter()
            .filter(|event| kind_of(event) == "user")
            .count(),
        2
    );
}

/// The thread's key decides which side is stored as `user`, so the same pair
/// reads the same way up whichever of them asked.
#[tokio::test]
async fn the_thread_is_stored_the_same_way_up_whoever_asked() {
    let room = room("orient", Fake::new(Scripted::new(answers("a1", "on it"))));
    room.deliver("bob", "ada", "can you look?").await.unwrap();

    let events = thread_of(&room, "ada~bob");
    assert_eq!(
        kinds(&events),
        ["agent", "thought", "user", "turn"],
        "ada is the key's first participant, so ada's words are the user side"
    );
    assert_eq!(events[0]["text"], "can you look?");
    assert_eq!(events[0]["receipt"], "read");
    assert_eq!(events[2]["text"], "on it");
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
    assert_eq!(marker_on(&room, "ada")["status"], "done");
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

/// A peer session has no conversation of its own to be answered in, so what
/// it sends a third teammate waits for the reply as before.
#[tokio::test]
async fn a_peer_session_still_waits_for_its_answer() {
    let room = room(
        "tool-peer",
        Fake::new(Scripted::new(answers("a1", "the winch"))),
    );
    let answered = TeammateTools::new(&room, "ada")
        .for_peer()
        .call(
            "message_teammate",
            &serde_json::json!({ "to": "Bob", "message": "what broke?" }),
        )
        .await
        .unwrap();

    let answered: Value = serde_json::from_str(&answered).unwrap();
    assert_eq!(answered["from"], "Bob");
    assert_eq!(answered["reply"], "the winch");
    assert!(
        room.tape("ada")
            .iter()
            .all(|event| kind_of(event) != "delivery")
    );
}

/// What a restart left: a delivery the agent never read is handed to it
/// again, once, and an exchange cut off mid-turn is closed and its sender
/// told, while an old one is only closed.
#[tokio::test]
async fn a_restart_hands_on_unheard_deliveries_and_closes_cut_off_exchanges() {
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
        },
    );
    for (id, ts) in [
        ("xthread:recent", now - 60_000),
        ("xthread:old", now - 3 * 60 * 60_000),
    ] {
        for (whose, other, name, role) in [
            ("ada", "bob", "Bob", PeerRole::Caller),
            ("bob", "ada", "Ada", PeerRole::Target),
        ] {
            room.write(
                whose,
                &TranscriptEvent::Peer {
                    id: id.to_string(),
                    ts,
                    thread_key: if id.ends_with("recent") {
                        "ada~bob"
                    } else {
                        "ada~old"
                    }
                    .to_string(),
                    with_persona_id: other.to_string(),
                    with_name: name.to_string(),
                    role,
                    exchanges: 0,
                    status: PeerStatus::Open,
                    seat: None,
                },
            );
        }
    }

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
    for whose in ["ada", "bob"] {
        let tape = room.tape(whose);
        for id in ["xthread:recent", "xthread:old"] {
            let marker = tape.iter().rev().find(|event| event["id"] == id).unwrap();
            assert_eq!(marker["status"], "failed", "{whose} {id}");
        }
    }
    let told = delivered(&room, "ada", 3).await;
    assert_eq!(told["cause"]["status"], "failed");
    assert_eq!(told["cause"]["personaId"], "bob");
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|event| kind_of(event) == "delivery" && event["cause"]["status"] == "failed")
            .count(),
        1,
        "only the recent exchange is worth waking for"
    );
}

#[tokio::test]
async fn a_teammate_this_room_does_not_have_is_refused_in_a_sentence() {
    let room = room("stranger", Fake::new(Scripted::new(answers("a1", "aye"))));
    let refused = room.deliver("ada", "Cal", "hello?").await.unwrap_err();
    assert!(refused.contains("Cal"), "{refused}");
    assert!(refused.contains("list_teammates"), "{refused}");
    assert!(thread_of(&room, "ada~cal").is_empty());

    let itself = room.deliver("ada", "Ada", "hello?").await.unwrap_err();
    assert!(itself.contains("cannot message itself"), "{itself}");
}

#[tokio::test]
async fn a_thread_is_listed_for_both_sides_and_its_reply_can_be_read() {
    let room = room("listed", Fake::new(Scripted::new(answers("a1", "aye"))));
    room.deliver("ada", "bob", "are you free?").await.unwrap();

    let listed = room.peer_threads("ada");
    assert_eq!(listed.len(), 1);
    let summary = &listed[0];
    assert_eq!(summary.thread_key, "ada~bob");
    assert_eq!(summary.with_persona_id, "bob");
    assert_eq!(summary.with_name, "Bob");
    assert_eq!(summary.exchanges, 1);
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

#[tokio::test]
async fn a_peer_session_that_has_gone_quiet_is_stopped_and_a_deleted_teammate_takes_its_own() {
    let room = room("idle", Fake::new(Scripted::new(answers("a1", "aye"))));
    room.deliver("ada", "bob", "still there?").await.unwrap();
    assert_eq!(lock(&room.peers.sessions).len(), 1);

    let now = now_ms();
    room.sweep_peers(now);
    assert_eq!(
        lock(&room.peers.sessions).len(),
        1,
        "a session used a moment ago is not idle"
    );
    for live in lock(&room.peers.sessions).values() {
        *lock(&live.last_used) = now - IDLE_MS - 1;
    }
    room.sweep_peers(now);
    assert!(lock(&room.peers.sessions).is_empty());

    room.deliver("ada", "bob", "and now?").await.unwrap();
    assert_eq!(lock(&room.peers.sessions).len(), 1);
    room.drop_peer_sessions("bob");
    assert!(lock(&room.peers.sessions).is_empty());
}

/// A peer-only session has two authorities: the caller is allowed to ask, and
/// the target is allowed to answer. Dropping either side removes the cached
/// driver and revokes the handles cloned into it, while a fresh lease remains
/// usable after that side is reattached.
#[tokio::test]
async fn invalidating_either_side_revokes_cached_peer_tools_without_a_main_session() {
    let room = room(
        "invalidate-peer",
        Fake::new(Scripted::turns(vec![
            answers("a1", "first"),
            answers("a2", "second"),
        ])),
    );

    room.deliver("ada", "bob", "first question").await.unwrap();
    let first = {
        lock(&room.peers.sessions)
            .get(&(String::from("ada"), String::from("bob")))
            .cloned()
            .expect("the first peer session is cached")
    };
    let first_target_tools =
        TeammateTools::new(&room, "bob").with_capability(first.target_capability.clone());
    assert!(
        first_target_tools
            .call("list_teammates", &json!({}))
            .await
            .is_ok()
    );

    // There is no main session to stop: revoking the caller still has to
    // invalidate the target driver and every tool handle it owns.
    room.invalidate("ada").unwrap();
    assert!(lock(&room.peers.sessions).is_empty());
    let refused = first_target_tools
        .call("list_teammates", &json!({}))
        .await
        .unwrap_err();
    assert!(
        refused.contains("capabilities have been revoked"),
        "{refused}"
    );

    room.reattach("ada").await.unwrap();
    let fresh_target_tools =
        TeammateTools::new(&room, "bob").with_capability(room.capability_lease("bob"));
    assert!(
        fresh_target_tools
            .call("list_teammates", &json!({}))
            .await
            .is_ok()
    );

    room.deliver("ada", "bob", "second question").await.unwrap();
    let second = {
        lock(&room.peers.sessions)
            .get(&(String::from("ada"), String::from("bob")))
            .cloned()
            .expect("the replacement peer session is cached")
    };
    let second_target_tools =
        TeammateTools::new(&room, "bob").with_capability(second.target_capability.clone());

    room.invalidate("bob").unwrap();
    assert!(lock(&room.peers.sessions).is_empty());
    let refused = second_target_tools
        .call("list_teammates", &json!({}))
        .await
        .unwrap_err();
    assert!(
        refused.contains("capabilities have been revoked"),
        "{refused}"
    );

    room.reattach("bob").await.unwrap();
    let fresh_target_tools =
        TeammateTools::new(&room, "bob").with_capability(room.capability_lease("bob"));
    assert!(
        fresh_target_tools
            .call("list_teammates", &json!({}))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn peer_teardown_does_not_revoke_the_callers_main_tools_but_main_stop_does() {
    let room = workspace_room(
        "peer-lease-ownership",
        Fake::new(Scripted::turns(vec![
            answers("a1", "first"),
            answers("a2", "second"),
        ])),
    );
    room.start("ada").await.unwrap();
    let main_capability = room.session("ada").unwrap().capability.clone();
    let main_tools = TeammateTools::new(&room, "ada").with_capability(main_capability);
    assert!(main_tools.call("list_teammates", &json!({})).await.is_ok());

    let first = {
        let tools = main_tools.clone();
        tokio::spawn(async move {
            tools
                .call(
                    "message_teammate",
                    &json!({ "to": "bob", "message": "one" }),
                )
                .await
        })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    let first_result: Value = serde_json::from_str(&first.await.unwrap().unwrap()).unwrap();
    assert_eq!(first_result["sent"], true);
    assert_eq!(delivered(&room, "ada", 1).await["text"], "first");

    let first_peer = lock(&room.peers.sessions)
        .get(&(String::from("ada"), String::from("bob")))
        .cloned()
        .expect("the peer session is cached");
    let first_peer_tools =
        TeammateTools::new(&room, "bob").with_capability(first_peer.target_capability.clone());
    room.start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert!(
        main_tools.call("list_teammates", &json!({})).await.is_ok(),
        "ending a peer must leave the main caller lease usable"
    );
    assert!(
        first_peer_tools
            .call("list_teammates", &json!({}))
            .await
            .is_err()
    );
    // The answer to the next message is delivered into Ada's conversation,
    // and a delivery after a chapter closed opens the next one on a fresh
    // session, as the person's next message would. Open it first, so what is
    // under test is the peer session and not that restart.
    room.in_this_chapter("ada").await.unwrap();
    let main_tools = TeammateTools::new(&room, "ada")
        .with_capability(room.session("ada").unwrap().capability.clone());

    let second = {
        let tools = main_tools.clone();
        tokio::spawn(async move {
            tools
                .call(
                    "message_teammate",
                    &json!({ "to": "bob", "message": "two" }),
                )
                .await
        })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    assert!(second.await.unwrap().is_ok());
    delivered(&room, "ada", 2).await;
    let second_peer = lock(&room.peers.sessions)
        .get(&(String::from("ada"), String::from("bob")))
        .cloned()
        .expect("the replacement peer session is cached");
    let second_peer_tools =
        TeammateTools::new(&room, "bob").with_capability(second_peer.target_capability.clone());

    room.stop("ada").unwrap();
    assert!(main_tools.call("list_teammates", &json!({})).await.is_err());
    assert!(
        second_peer_tools
            .call("list_teammates", &json!({}))
            .await
            .is_err()
    );
    assert!(lock(&room.peers.sessions).is_empty());
}

#[tokio::test]
async fn a_dropped_collaboration_wait_is_expired_and_cannot_be_answered_later() {
    let room = workspace_room(
        "dropped-collaboration-wait",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "wait for approval").await })
    };
    let card = collaboration_card(&room, "ada").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    delivery.abort();
    assert!(delivery.await.unwrap_err().is_cancelled());
    for _ in 0..100 {
        if lock(&room.peers.waiting).is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(lock(&room.peers.waiting).is_empty());
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
        .expect("cancellation leaves an expired collaboration card");
    assert_eq!(settled["decision"], "expired");
    assert!(room.persona("bob").unwrap().allowed_senders.is_empty());
}

#[tokio::test]
async fn an_answered_collaboration_cannot_start_after_a_chapter_generation_changes() {
    let room = workspace_room(
        "collaboration-generation-race",
        Fake::new(Scripted::new(answers("a1", "should not run"))),
    );
    let delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "start after approval").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    room.peers.advance_generation("ada");

    let error = delivery.await.unwrap().unwrap_err();
    assert!(error.contains("approval expired"), "{error}");
    assert!(thread_of(&room, "ada~bob").is_empty());
    assert!(lock(&room.peers.sessions).is_empty());
    assert!(lock(&room.peers.session_grants).is_empty());
}

#[tokio::test]
async fn nested_peer_leases_follow_the_outer_target_but_revoke_independently() {
    let room = workspace_room(
        "nested-peer-leases",
        Fake::new(Scripted::turns(vec![
            answers("a1", "outer"),
            answers("a2", "nested"),
        ])),
    );
    let outer_delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "outer question").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    outer_delivery.await.unwrap().unwrap();
    let outer = lock(&room.peers.sessions)
        .get(&(String::from("ada"), String::from("bob")))
        .cloned()
        .expect("the outer peer session is cached");

    let nested_delivery = {
        let room = room.clone();
        let caller_capability = outer.target_capability.clone();
        tokio::spawn(async move {
            room.deliver_with_capability("bob", "ada", "nested question", Some(caller_capability))
                .await
        })
    };
    let card = collaboration_card(&room, "bob").await;
    room.answer_permission("bob", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    nested_delivery.await.unwrap().unwrap();
    let nested = lock(&room.peers.sessions)
        .get(&(String::from("bob"), String::from("ada")))
        .cloned()
        .expect("the nested peer session is cached");

    // Ending B's nested conversation only revokes its scoped caller lease.
    nested.caller_capability.revoke();
    assert!(outer.target_capability.is_current());
    assert!(outer.valid());

    // Revoking the outer A->B target lease propagates through the nested
    // caller lease, even though the nested session has its own token.
    outer.target_capability.revoke();
    assert!(!nested.valid());
}

#[tokio::test]
async fn revoking_an_outer_peer_settles_a_nested_collaboration_wait() {
    let room = workspace_room(
        "nested-peer-wait",
        Fake::new(Scripted::new(answers("a1", "outer"))),
    );
    let outer_delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "outer question").await })
    };
    let card = collaboration_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), ALLOW_SESSION)
        .await
        .unwrap();
    outer_delivery.await.unwrap().unwrap();
    let outer = lock(&room.peers.sessions)
        .get(&(String::from("ada"), String::from("bob")))
        .cloned()
        .expect("the outer peer session is cached");

    let nested_delivery = {
        let room = room.clone();
        let caller_capability = outer.target_capability.clone();
        tokio::spawn(async move {
            room.deliver_with_capability("bob", "ada", "nested question", Some(caller_capability))
                .await
        })
    };
    let card = collaboration_card(&room, "bob").await;
    let request_id = card["requestId"].as_str().unwrap().to_string();
    outer.target_capability.revoke();
    room.settle_invalid_collaboration();

    assert!(nested_delivery.await.unwrap().is_err());
    assert!(lock(&room.peers.waiting).is_empty());
    let settled = room
        .tape("bob")
        .into_iter()
        .find(|event| event["requestId"] == request_id)
        .expect("the nested card is settled when the outer lease ends");
    assert_eq!(settled["decision"], "expired");
}

#[tokio::test]
async fn revocation_reaches_a_third_teammates_delegated_tools_but_not_its_main_session() {
    let agents = Fake::new(Scripted::turns(vec![
        answers("a1", "outer"),
        answers("a2", "nested"),
    ]));
    let room = room("three-party-revocation", agents.clone());
    let mut cal = persona("cal");
    cal.name = "Cal".to_string();
    enrol(room.log(), &cal);
    for id in ["ada", "bob", "cal"] {
        room.start(id).await.unwrap();
    }
    let ada = TeammateTools::new(&room, "ada")
        .with_capability(room.session("ada").unwrap().capability.clone());
    let bob_main = TeammateTools::new(&room, "bob")
        .with_capability(room.session("bob").unwrap().capability.clone());
    let cal_main = TeammateTools::new(&room, "cal")
        .with_capability(room.session("cal").unwrap().capability.clone());
    ada.call(
        "message_teammate",
        &json!({"to": "bob", "message": "outer"}),
    )
    .await
    .unwrap();
    delivered(&room, "ada", 1).await;
    let outer = lock(&room.peers.sessions)[&("ada".to_string(), "bob".to_string())].clone();
    let bob_delegated = TeammateTools::new(&room, "bob")
        .with_capability(outer.target_capability.clone())
        .for_peer();
    bob_delegated
        .call(
            "message_teammate",
            &json!({"to": "cal", "message": "nested"}),
        )
        .await
        .unwrap();
    let nested = lock(&room.peers.sessions)[&("bob".to_string(), "cal".to_string())].clone();
    let cal_delegated =
        TeammateTools::new(&room, "cal").with_capability(nested.target_capability.clone());
    assert!(
        cal_delegated
            .call("list_teammates", &json!({}))
            .await
            .is_ok()
    );
    let cancellations = agents.cancel_count();

    room.stop("ada").unwrap();

    assert!(
        cal_delegated
            .call("list_teammates", &json!({}))
            .await
            .is_err()
    );
    assert!(
        bob_delegated
            .call("list_teammates", &json!({}))
            .await
            .is_err()
    );
    assert!(bob_main.call("list_teammates", &json!({})).await.is_ok());
    assert!(cal_main.call("list_teammates", &json!({})).await.is_ok());
    assert!(lock(&room.peers.sessions).is_empty());
    assert_eq!(
        agents.cancel_count() - cancellations,
        3,
        "stop the main caller and both delegated drivers"
    );
}

/// The receipt machine, which is what the two ticks in a thread mean.
///
/// Every one of these is about the *kind* of event and never its text: a tick
/// the model could produce by writing the right sentence would be a lie the
/// reader has no way to check.
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
        }
    }

    fn thought(text: &str) -> TranscriptEvent {
        TranscriptEvent::Thought {
            id: "th".to_string(),
            ts: 3,
            text: text.to_string(),
        }
    }

    fn tool() -> TranscriptEvent {
        TranscriptEvent::Tool {
            id: "to".to_string(),
            ts: 4,
            tool_call_id: "c1".to_string(),
            title: "ls .".to_string(),
            tool_kind: None,
            status: ToolStatus::Completed,
            locations: None,
            output: None,
        }
    }

    fn turn() -> TranscriptEvent {
        TranscriptEvent::Turn {
            id: "tu".to_string(),
            ts: 5,
            stop_reason: "end_turn".to_string(),
            usage: None,
        }
    }

    fn notice(text: &str) -> TranscriptEvent {
        TranscriptEvent::Notice {
            id: "no".to_string(),
            ts: 6,
            level: NoticeLevel::Error,
            text: text.to_string(),
        }
    }

    fn chapter() -> TranscriptEvent {
        TranscriptEvent::Chapter {
            id: "ch".to_string(),
            ts: 7,
            backend_id: "hotline".to_string(),
            session_id: None,
            ended_at: None,
            title: None,
            note: None,
            status: None,
            tags: None,
            closed_by: None,
            resumed_from: None,
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

    /// A whole delivery through the seam, and what it wrote.
    struct Ran {
        window: Option<TranscriptEvent>,
        stored: Vec<TranscriptEvent>,
        reads: Vec<String>,
    }

    fn run(events: Vec<TranscriptEvent>) -> Ran {
        let mut window = None;
        let mut stored = Vec::new();
        let mut reads = Vec::new();
        for event in events {
            let step = through_receipts(&mut window, event);
            if let Some(read) = step.read {
                reads.push(id_of(&read));
            }
            stored.push(step.event);
        }
        Ran {
            window,
            stored,
            reads,
        }
    }

    #[test]
    fn a_message_entering_the_thread_is_sent() {
        let ran = run(vec![user("u1", "hello")]);
        assert_eq!(receipt(&ran.stored[0]), Some(Receipt::Sent));
    }

    #[test]
    fn the_reply_is_sent_too() {
        let ran = run(vec![user("u1", ""), agent("a1", "hi")]);
        assert_eq!(receipt(&ran.stored[1]), Some(Receipt::Sent));
    }

    #[test]
    fn the_targets_first_sign_of_a_turn_reads_the_callers_message() {
        let ran = run(vec![user("u1", ""), thought("…")]);
        assert_eq!(ran.reads, ["u1"]);
    }

    #[test]
    fn read_is_stamped_once_not_on_every_event_of_the_turn() {
        let ran = run(vec![
            user("u1", ""),
            thought(""),
            tool(),
            agent("a1", ""),
            turn(),
        ]);
        assert_eq!(ran.reads, ["u1"]);
    }

    #[test]
    fn the_reply_alone_is_proof_enough() {
        let ran = run(vec![user("u1", ""), agent("a1", "done")]);
        assert_eq!(ran.reads, ["u1"]);
    }

    #[test]
    fn a_turn_that_stopped_with_nothing_to_say_still_read_the_message() {
        let ran = run(vec![user("u1", ""), turn()]);
        assert_eq!(ran.reads, ["u1"]);
    }

    #[test]
    fn an_error_before_the_model_ran_does_not_read_the_message() {
        let ran = run(vec![user("u1", ""), notice("the backend died")]);
        assert!(ran.reads.is_empty());
        assert_eq!(ran.window.as_ref().map(id_of), Some("u1".to_string()));
    }

    #[test]
    fn a_chapter_marker_written_as_the_session_opens_does_not_read_the_message() {
        let ran = run(vec![user("u1", ""), chapter()]);
        assert!(ran.reads.is_empty());
    }

    #[test]
    fn nothing_is_read_before_a_message_arrives() {
        let ran = run(vec![thought(""), tool(), agent("a1", ""), turn()]);
        assert!(ran.reads.is_empty());
    }

    #[test]
    fn two_deliveries_each_get_their_own_read() {
        let ran = run(vec![
            user("u1", ""),
            agent("a1", ""),
            turn(),
            user("u2", ""),
            agent("a2", ""),
            turn(),
        ]);
        assert_eq!(ran.reads, ["u1", "u2"]);
    }

    /// A caller that sent twice in a row: the ticks belong to the message the
    /// turn is actually about, and the earlier one waits for its own.
    #[test]
    fn a_second_message_before_any_turn_supersedes_the_one_waiting() {
        let ran = run(vec![user("u1", ""), user("u2", ""), agent("a1", "")]);
        assert_eq!(ran.reads, ["u2"]);
    }

    #[test]
    fn the_text_of_the_events_is_never_read() {
        for wording in [
            "",
            "read",
            "I have read your message",
            "receipt: read",
            "✓✓",
        ] {
            let ran = run(vec![user("u1", wording), thought(wording)]);
            assert_eq!(ran.reads, ["u1"], "{wording}");
            // …and the same sentence with no turn behind it stays unread.
            let ran = run(vec![user("u1", wording), notice(wording)]);
            assert!(ran.reads.is_empty(), "{wording}");
        }
    }

    #[test]
    fn the_ladder_only_climbs() {
        assert_eq!(higher(None, Receipt::Sent), Receipt::Sent);
        assert_eq!(higher(Some(Receipt::Sent), Receipt::Read), Receipt::Read);
        assert_eq!(higher(Some(Receipt::Read), Receipt::Sent), Receipt::Read);
        assert_eq!(higher(Some(Receipt::Read), Receipt::Read), Receipt::Read);
        // A message that already carries a receipt is not lowered by the fold.
        let already = stamped(user("u1", ""), Receipt::Read);
        let ran = run(vec![already]);
        assert_eq!(receipt(&ran.stored[0]), Some(Receipt::Read));
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
        assert!(read_receipt_updates(&said, &["a1".to_string()]).is_empty());
        assert!(read_receipt_updates(&said, &["no-such-id".to_string()]).is_empty());
        assert!(read_receipt_updates(&[], &["anything".to_string()]).is_empty());
    }

    #[test]
    fn a_receipt_cannot_move_machinery() {
        let machinery = stored(&[thought("")]);
        assert!(read_receipt_updates(&machinery, &["th".to_string()]).is_empty());
    }
}

/// A permission raised inside a peer turn is on a stream no seat draws a
/// card for, so nobody ever answers it; when the turn ends the card is
/// expired on the thread the way a tape's would be, and the thread stops
/// saying somebody is waiting.
#[tokio::test]
async fn a_permission_left_open_in_a_peer_turn_is_expired_when_the_turn_ends() {
    let mut script = vec![Update::Permission {
        request_id: "r1".to_string(),
        title: "Run ls".to_string(),
        options: vec![crate::contract::PermissionOption {
            option_id: "once".to_string(),
            name: "Allow once".to_string(),
            kind: None,
        }],
    }];
    script.extend(answers("a1", "aye"));
    let room = room("peer-permission", Fake::new(Scripted::new(script)));

    room.deliver("ada", "Bob", "may I look?").await.unwrap();

    let events = thread_of(&room, "ada~bob");
    let card = events
        .iter()
        .find(|event| kind_of(event) == "permission")
        .expect("the card is on the thread");
    assert_eq!(card["decision"], "expired", "{card}");
    assert!(
        room.peer_threads("ada")
            .iter()
            .all(|thread| !thread.waiting),
        "a thread whose turn ended still says somebody is waiting"
    );
}
