//! A `send` that raised a first-contact card waits for the person's answer,
//! briefly, so the teammate's one message about it is true
//! (`docs/design/agent-inbox.md`, The tools): yes in time is "in inbox" as a
//! send that needed no card says it, no is "declined" with nothing sent,
//! and no answer by the wait's end is "not sent yet". The wait ends with the
//! call, leaves the card and the item as they were, and holds up nobody
//! else's inbox or the card's own answer.

use super::*;
use hotline_voice::cards::Card;

/// Ada works in her own folders; Bob has the whole machine, more than she
/// has, so her items to him wait on the person's card; Cy works in her own
/// folders as Ada does, so either reaches her without one.
fn up(name: &str) -> Arc<Room> {
    let log = scratch(name);
    enrol(&log, &named("ada", "Ada", None));
    enrol(&log, &named("bob", "Bob", Some(Reach::Machine)));
    enrol(&log, &named("cy", "Cy", None));
    Room::with_agents(log, Arc::new(DeskKeys), Fake::new(replies()))
}

/// Ada's `send`, called as her agent calls it, on a task of its own so the
/// test can answer the card it waits on.
fn sending(room: &Arc<Room>, arguments: Value) -> tokio::task::JoinHandle<Result<Value, String>> {
    let room = room.clone();
    tokio::spawn(async move { tool(&room, "ada", "send", arguments).await })
}

/// The first-contact card up on Ada's tape, by its request id, once it is.
async fn card(room: &Room) -> String {
    let open = || {
        room.tape("ada").into_iter().find(|event| {
            event["kind"] == "permission"
                && event["requestId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(crate::session::peers::COLLAB_REQUEST_PREFIX))
                && event.get("decision").is_none()
        })
    };
    until("the first-contact card", || open().is_some()).await;
    open().unwrap()["requestId"].as_str().unwrap().to_string()
}

fn still_up(room: &Room, request: &str) -> bool {
    room.tape("ada")
        .iter()
        .any(|event| event["requestId"] == request && event.get("decision").is_none())
}

fn id_of(sent: &Value, at: usize) -> String {
    sent["sent"][at]["id"].as_str().unwrap().to_string()
}

fn note_of(sent: &Value) -> String {
    sent["note"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_yes_inside_the_wait_says_in_inbox_as_a_send_without_a_card_does_and_lands_once() {
    let room = up("held-yes");
    let send = sending(
        &room,
        json!({"to": ["bob"], "title": "Psst", "message": "the secret"}),
    );
    let request = card(&room).await;
    assert!(!send.is_finished(), "the send waits on the card");
    room.answer_permission("ada", &request, "allow")
        .await
        .unwrap();
    let sent = send.await.unwrap().unwrap();
    assert_eq!(sent["delivery"], "Bob: in inbox", "{sent}");
    assert_eq!(sent["sent"][0]["delivery"], "in inbox");
    assert_eq!(note_of(&sent), format!("Bob: in inbox. {IN_INBOX}"));

    // The same words a send that needed no card says.
    let unheld = tool(&room, "ada", "send", json!({"to": ["cy"], "title": "Hi"}))
        .await
        .unwrap();
    assert_eq!(note_of(&unheld), format!("Cy: in inbox. {IN_INBOX}"));

    let id = id_of(&sent, 0);
    let landed = item(&room, "bob", &id);
    assert!(!landed.held);
    assert_eq!(
        states(&landed)
            .iter()
            .filter(|state| **state == InboxState::New)
            .count(),
        1,
        "it landed once"
    );
    assert_eq!(
        kept::received(&room.log, "bob")
            .iter()
            .filter(|item| item.id == id)
            .count(),
        1
    );
    assert_eq!(
        landed.sender_told, 1,
        "the result told Ada, so her next turn does not"
    );
}

#[tokio::test]
async fn a_no_says_declined_and_nothing_lands() {
    let room = up("held-no");
    let send = sending(
        &room,
        json!({"to": ["bob"], "title": "Psst", "message": "the secret"}),
    );
    let request = card(&room).await;
    room.answer_permission("ada", &request, "deny")
        .await
        .unwrap();
    let sent = send.await.unwrap().unwrap();
    assert_eq!(sent["delivery"], "Bob: not sent, the person declined");
    assert_eq!(
        note_of(&sent),
        "Bob: not sent, the person declined. The person declined your reaching Bob: nothing was sent to them, and the item was closed as dropped, unseen. Do not send it again, and do not reach Bob another way for it, through another colleague or a file. If you say anything of it, say in one line that you did not send it."
    );
    assert_eq!(kept::received(&room.log, "bob"), [] as [InboxItem; 0]);
    let dropped = item(&room, "bob", &id_of(&sent, 0));
    assert!(dropped.held, "Bob never had it");
    assert_eq!(dropped.outcome, Some(InboxOutcome::Dropped));
    assert_eq!(dropped.sender_told, 1);
}

/// A card taken down unanswered, here by the person's Stop of the two
/// teammates' exchange, is not the person's no: the result says not sent
/// and why, and never that they declined.
#[tokio::test]
async fn a_card_taken_down_unanswered_is_not_told_as_the_persons_no() {
    let room = up("held-stopped");
    let send = sending(&room, json!({"to": ["bob"], "title": "Psst"}));
    card(&room).await;
    room.settle_pair_collaboration("ada", "bob");
    let sent = send.await.unwrap().unwrap();
    assert_eq!(sent["delivery"], "Bob: not sent");
    let note = note_of(&sent);
    assert!(
        note.contains(
            "Nothing was sent to Bob: The card was taken down before the person answered it. The item was closed as dropped, unseen."
        ),
        "{note}"
    );
    assert!(!note.contains("declined"), "{note}");
}

#[tokio::test]
async fn unanswered_when_the_wait_ends_says_not_sent_yet_and_what_to_say() {
    let room = up("held-cap");
    tokio::time::pause();
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "title": "Psst", "message": "the secret"}),
    )
    .await
    .unwrap();
    assert_eq!(
        sent["delivery"],
        "Bob: not sent yet, waiting on the person's card"
    );
    let note = note_of(&sent);
    assert_eq!(
        note,
        "Bob: not sent yet, waiting on the person's card. Nothing was sent to Bob yet: the person has not yet allowed you to reach them, and a card asks them. Do not say it was sent or delivered. Say one short line that a card needs their answer before you can do what this was for, such as \"A card needs your answer before I can ask Bob.\", and nothing else about it. It goes to Bob by itself once they allow it, and you are told at the start of your next turn; do not send it again. If the person asks where it stands, `read_inbox` with list `sent` says."
    );
    assert!(!note.contains("in inbox"), "{note}");

    // The card and the item outlive the send: a later yes lets it in, and
    // Ada is told at her next turn, not by a result she already had.
    let request = card(&room).await;
    let id = id_of(&sent, 0);
    assert!(item(&room, "bob", &id).held);
    // The start's recovery, which comes round a few seconds after the room
    // opens, finds the item held and the card already waited on: it leaves
    // both alone rather than raising a second card that drops the item.
    room.recover_exchanges().await;
    assert_ne!(item(&room, "bob", &id).state, InboxState::Closed);
    room.answer_permission("ada", &request, "allow")
        .await
        .unwrap();
    until("it lands", || !item(&room, "bob", &id).held).await;
    assert_eq!(item(&room, "bob", &id).sender_told, 0);
}

/// A call turn is told the call announces the card, so it says nothing of
/// the card beyond waiting: the person would hear it twice.
#[tokio::test]
async fn on_a_call_the_held_note_leaves_the_card_to_the_call() {
    let room = up("held-call");
    tokio::time::pause();
    let sending =
        Sending::from_arguments(&json!({"to": ["bob"], "title": "Psst", "message": "?"})).unwrap();
    let sent: Value = serde_json::from_str(
        &room
            .send_items("ada", &ThreadId::call("call-1"), &sending, None)
            .await
            .unwrap(),
    )
    .unwrap();
    let note = note_of(&sent);
    assert!(
        note.contains("This turn is on a call, and the call announces the card itself: do not describe the card or read it out, and say at most that you are waiting for their answer."),
        "{note}"
    );
    assert!(!note.contains("Say one short line"), "{note}");
    assert!(
        note.contains("Do not say it was sent or delivered."),
        "{note}"
    );
}

/// What a call says of the card a held send raised: who wants to reach
/// whom, from the wait behind it, never the card's paragraph on access.
#[tokio::test]
async fn a_call_says_the_first_contact_card_short() {
    let room = up("held-spoken");
    let send = sending(&room, json!({"to": ["bob"], "title": "Psst"}));
    let request = card(&room).await;
    let card = Card {
        persona: "ada".into(),
        kind: "permission".into(),
        request,
    };
    let said = room.call_card_notice(&card, false).unwrap();
    assert!(said.answerable);
    assert_eq!(
        said.text,
        "Ada wants to reach Bob. Say yes to allow it this once, or no to decline."
    );
    assert_eq!(
        room.call_card_notice(&card, true).unwrap().text,
        "I want to reach Bob. Say yes to allow it this once, or no to decline."
    );
    send.abort();
}

/// The person's Interrupt, or a stop, ends the agent's call: the wait goes
/// with it at once, and the card and the item stay as they were, so a later
/// yes still lets it in.
#[tokio::test]
async fn ending_the_call_ends_the_wait_and_leaves_the_card_and_the_item() {
    let room = up("held-cancel");
    let send = sending(
        &room,
        json!({"to": ["bob"], "title": "Psst", "message": "the secret"}),
    );
    let request = card(&room).await;
    send.abort();
    assert!(send.await.unwrap_err().is_cancelled());
    let held = kept::of(&room.log, "bob");
    assert_eq!(held.len(), 1);
    assert!(held[0].held && held[0].state != InboxState::Closed);
    assert!(still_up(&room, &request), "the card is still the person's");
    room.answer_permission("ada", &request, "allow")
        .await
        .unwrap();
    until("it lands", || !item(&room, "bob", &held[0].id).held).await;
}

/// While Ada's send waits on the card, a colleague's inbox works as ever,
/// and the card's own answer goes through: the wait holds nothing they need.
#[tokio::test]
async fn a_waiting_send_holds_up_neither_another_teammates_inbox_nor_its_card() {
    let room = up("held-no-block");
    let send = sending(
        &room,
        json!({"to": ["bob"], "title": "Psst", "message": "the secret"}),
    );
    let request = card(&room).await;
    let prompt = Duration::from_secs(5);
    let to_cy = tokio::time::timeout(
        prompt,
        tool(
            &room,
            "bob",
            "send",
            json!({"to": ["cy"], "title": "Lunch?"}),
        ),
    )
    .await
    .expect("Bob's send is not held up")
    .unwrap();
    assert_eq!(to_cy["delivery"], "Cy: in inbox");
    let listed = tokio::time::timeout(prompt, tool(&room, "cy", "read_inbox", json!({})))
        .await
        .expect("Cy's inbox is not held up")
        .unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1, "{listed}");
    tokio::time::timeout(
        prompt,
        tool(
            &room,
            "cy",
            "update_inbox",
            json!({"items": [{"id": id_of(&to_cy, 0), "state": "closed", "outcome": "noted"}]}),
        ),
    )
    .await
    .expect("Cy's sorting is not held up")
    .unwrap();
    assert!(!send.is_finished(), "Ada's send still waits");
    tokio::time::timeout(prompt, room.answer_permission("ada", &request, "allow"))
        .await
        .expect("the card's answer is not held up")
        .unwrap();
    let sent = tokio::time::timeout(prompt, send)
        .await
        .expect("the answer ends the wait")
        .unwrap()
        .unwrap();
    assert_eq!(sent["delivery"], "Bob: in inbox");
}

/// A send to several waits on the one card it raised and says each
/// recipient's line: in for the one who needed no card, declined for the
/// one the person said no to.
#[tokio::test]
async fn a_group_send_with_one_held_recipient_says_each_ones_line() {
    let room = up("held-group");
    let send = sending(
        &room,
        json!({"to": ["cy", "bob"], "title": "Standup moved", "message": "To ten."}),
    );
    let request = card(&room).await;
    assert!(!item(&room, "cy", &kept::received(&room.log, "cy")[0].id).held);
    room.answer_permission("ada", &request, "deny")
        .await
        .unwrap();
    let sent = send.await.unwrap().unwrap();
    assert_eq!(
        sent["delivery"],
        "Cy: in inbox · Bob: not sent, the person declined"
    );
    assert_eq!(sent["sent"][0]["delivery"], "in inbox");
    assert_eq!(sent["sent"][1]["delivery"], "not sent, the person declined");
    let note = note_of(&sent);
    assert!(
        note.starts_with(&format!(
            "Cy: in inbox · Bob: not sent, the person declined. {IN_INBOX} The person declined your reaching Bob:"
        )),
        "{note}"
    );
    assert_eq!(kept::received(&room.log, "bob"), [] as [InboxItem; 0]);
}
