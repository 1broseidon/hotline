//! Two teammates talking, driven by the scripted driver.
//!
//! Nothing here reaches a model. What is under test is what a delivery leaves
//! behind: the thread both sides share, the receipts on it, and the marker on
//! each of their own tapes.

use super::*;
use crate::driver::{MessageKind, Update};
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};

/// A room with Ada and Bob enrolled, neither of them running.
fn room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    let mut bob = persona("bob");
    bob.name = "Bob".to_string();
    enrol(&log, &bob);
    Room::with_agents(log, Arc::new(DeskKeys), agents)
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

#[tokio::test]
async fn the_tool_hands_the_caller_the_recipients_reply() {
    let room = room("tool", Fake::new(Scripted::new(answers("a1", "the winch"))));
    let answered = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &serde_json::json!({ "to": "Bob", "message": "what broke?" }),
        )
        .await
        .unwrap();

    let answered: Value = serde_json::from_str(&answered).unwrap();
    assert_eq!(answered["from"], "Bob");
    assert_eq!(answered["reply"], "the winch");
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
            backend_id: "pi".to_string(),
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
