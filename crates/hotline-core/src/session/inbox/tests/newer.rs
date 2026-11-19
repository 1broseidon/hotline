//! A desk opened by this build after a newer Hotline ran on it: what that
//! build wrote on an inbox item and this one does not read is left exactly
//! as it was found (`docs/log.md`, The room stream).
//!
//! No build newer than this one exists yet, so the fixtures stand for any: a
//! field on an item, on its receipts and inside the thread it names this
//! build does not know, and an item in a state or with an outcome no build
//! has yet.

use super::pick_up::reopened;
use super::*;
use crate::session::inbox::clock::expire_after;

/// What the newer build wrote on an item, and on each receipt it wrote.
const ITS_FIELD: (&str, &str) = ("lane", "quiet");
const ITS_RECEIPT_FIELD: (&str, &str) = ("by", "the newer build");

/// An item from Ada in Bob's inbox, kept as `shape` leaves it, as the newer
/// build wrote it: with a field of its own on the item, on each receipt and
/// inside the thread it was sent from.
/// It wakes nobody, so only what the test does moves it.
fn left_by_a_newer_build(
    room: &Room,
    title: &str,
    wants: InboxWants,
    shape: impl FnOnce(&mut InboxItem),
) -> String {
    let ada = room.persona("ada").unwrap();
    let mut kept = crate::session::inbox::item(
        &ada,
        &ThreadId::dm("ada"),
        "bob",
        title.into(),
        format!("the words of {title}"),
        wants,
    );
    moved(&mut kept, InboxState::New, None, None, None);
    kept.sender_told = 1;
    shape(&mut kept);
    let mut line = kept::line(&kept);
    line[ITS_FIELD.0] = json!(ITS_FIELD.1);
    line["fromThread"][ITS_FIELD.0] = json!(ITS_FIELD.1);
    for receipt in line["receipts"].as_array_mut().unwrap() {
        receipt[ITS_RECEIPT_FIELD.0] = json!(ITS_RECEIPT_FIELD.1);
    }
    room.log
        .append(&StreamId::Inbox("bob".into()), &line)
        .unwrap();
    kept.id
}

/// The latest line of an item in Bob's inbox, as it is on disk.
fn line_of(room: &Room, id: &str) -> Value {
    room.log
        .load(&StreamId::Inbox("bob".into()))
        .into_iter()
        .find(|line| line["id"] == id)
        .expect("the item is kept")
}

/// After `what` rewrote it, the item still carries the newer build's field,
/// and so do the `receipts` receipts it wrote; this build reads it, with the
/// field kept on it and the stream's own `kind` not.
fn kept_its_fields(room: &Room, id: &str, receipts: usize, what: &str) {
    let line = line_of(room, id);
    assert_eq!(
        line[ITS_FIELD.0], ITS_FIELD.1,
        "{what} dropped the item's field: {line}"
    );
    let carried = line["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|receipt| receipt[ITS_RECEIPT_FIELD.0] == ITS_RECEIPT_FIELD.1)
        .count();
    assert_eq!(
        carried, receipts,
        "{what} dropped a receipt's field: {line}"
    );
    assert_eq!(
        line["fromThread"][ITS_FIELD.0], ITS_FIELD.1,
        "{what} dropped the field inside the thread it was sent from: {line}"
    );
    let read = item(room, "bob", id);
    assert_eq!(read.newer.get(ITS_FIELD.0), Some(&json!(ITS_FIELD.1)));
    assert!(!read.newer.contains_key("kind"), "{:?}", read.newer);
}

/// Every path that rewrites an item keeps what a newer build wrote on it:
/// a turn offered it, reading it, later, taken in the turn, closed, an
/// answer, combined, taken into a work thread and its result, its sender
/// told, the person's card letting it in or saying no, expiry, and the
/// pick-up a desk that opens offers.
#[tokio::test]
async fn every_move_of_an_item_keeps_the_fields_a_newer_build_wrote_on_it() {
    let (room, _) = room("inbox-newer-fields");
    at_rest(&room, "ada").await;
    let sort = |entry: Value| {
        let room = room.clone();
        async move {
            tool(&room, "bob", "update_inbox", json!({ "items": [entry] }))
                .await
                .unwrap()
        }
    };

    let moving = left_by_a_newer_build(&room, "Moving", InboxWants::Answer, |_| {});
    room.inbox_told("bob", &ThreadId::dm("bob"), false);
    let offered = item(&room, "bob", &moving);
    assert!(offered.offered.is_some());
    assert_eq!(offered.state, InboxState::Noticed);
    kept_its_fields(&room, &moving, 1, "a turn offered it");
    tool(&room, "bob", "read_inbox", json!({ "ids": [moving] }))
        .await
        .unwrap();
    assert_eq!(item(&room, "bob", &moving).state, InboxState::Seen);
    kept_its_fields(&room, &moving, 1, "reading it");
    sort(json!({"id": moving, "state": "later", "reason": "after lunch"})).await;
    assert_eq!(item(&room, "bob", &moving).state, InboxState::Later);
    kept_its_fields(&room, &moving, 1, "later");
    sort(json!({"id": moving, "state": "taken", "in_thread": false})).await;
    assert_eq!(item(&room, "bob", &moving).state, InboxState::Taken);
    kept_its_fields(&room, &moving, 1, "taken in the turn");
    sort(json!({"id": moving, "state": "closed", "outcome": "noted"})).await;
    assert_eq!(
        item(&room, "bob", &moving).outcome,
        Some(InboxOutcome::Noted)
    );
    kept_its_fields(&room, &moving, 1, "closed");

    let answered = left_by_a_newer_build(&room, "Answered", InboxWants::Answer, |_| {});
    sort(json!({"id": answered, "answer": "Yes."})).await;
    assert_eq!(
        item(&room, "bob", &answered).outcome,
        Some(InboxOutcome::Answered)
    );
    kept_its_fields(&room, &answered, 1, "an answer");

    let folded = left_by_a_newer_build(&room, "Folded", InboxWants::Answer, |_| {});
    let kept_open = left_by_a_newer_build(&room, "Kept open", InboxWants::Answer, |_| {});
    sort(json!({"id": folded, "state": "combined", "into": kept_open})).await;
    assert_eq!(
        item(&room, "bob", &folded).outcome,
        Some(InboxOutcome::Combined)
    );
    kept_its_fields(&room, &folded, 1, "combined");

    let work = left_by_a_newer_build(&room, "Fix the crane", InboxWants::Work, |_| {});
    sort(json!({"id": work, "state": "taken"})).await;
    until("the work is done", || {
        item(&room, "bob", &work).state == InboxState::Closed
    })
    .await;
    let done = item(&room, "bob", &work);
    assert_eq!(done.outcome, Some(InboxOutcome::Done));
    assert!(done.work.is_some() && done.request.is_some());
    kept_its_fields(&room, &work, 1, "taken into a work thread and done");

    room.inbox_told("ada", &ThreadId::dm("ada"), false);
    for id in [&moving, &answered, &folded, &work] {
        let told = item(&room, "bob", id);
        assert_eq!(told.sender_told as usize, told.receipts.len(), "{id}");
        kept_its_fields(&room, id, 1, "its sender told");
    }

    let held = |item: &mut InboxItem| {
        item.held = true;
        item.receipts.clear();
        item.sender_told = 0;
    };
    let let_in = left_by_a_newer_build(&room, "Let in", InboxWants::Answer, held);
    room.consent_decided(&item(&room, "bob", &let_in), Ok(()));
    let landed = item(&room, "bob", &let_in);
    assert!(!landed.held && landed.state == InboxState::New);
    kept_its_fields(&room, &let_in, 0, "the person's card letting it in");
    let said_no = left_by_a_newer_build(&room, "Said no", InboxWants::Answer, held);
    room.consent_decided(
        &item(&room, "bob", &said_no),
        Err(String::from("No.").into()),
    );
    assert_eq!(
        item(&room, "bob", &said_no).outcome,
        Some(InboxOutcome::Dropped)
    );
    kept_its_fields(&room, &said_no, 0, "the person's card saying no");

    let stale = left_by_a_newer_build(&room, "Stale", InboxWants::Answer, |_| {});
    room.inbox_due(now_ms() + 10 * expire_after()).await;
    assert_eq!(
        item(&room, "bob", &stale).outcome,
        Some(InboxOutcome::Expired)
    );
    kept_its_fields(&room, &stale, 1, "expiry");

    let set_aside = left_by_a_newer_build(&room, "Set aside", InboxWants::Answer, |item| {
        item.offered = Some(now_ms());
        moved(
            item,
            InboxState::Later,
            Some("after the release".into()),
            None,
            None,
        );
    });
    let (room, _) = reopened(room);
    room.recover_exchanges().await;
    until("the pick-up offered it", || {
        item(&room, "bob", &set_aside).offer_again.is_some()
    })
    .await;
    kept_its_fields(&room, &set_aside, 2, "the pick-up at start");
}

/// The three moves of an item the test above leaves out keep what a newer
/// build wrote on it too: the notice a busy receiver is handed mid-turn, the
/// claim a take lets go of when its work could not be queued, and the
/// person's Stop of the work it was taken into.
#[tokio::test]
async fn the_notice_the_failed_take_and_the_persons_stop_keep_the_fields_a_newer_build_wrote() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(
        "inbox-newer-three",
        replies().gated(gate.clone()).consuming(),
    );

    // Bob is mid-turn when the item lands: it is noticed, not offered.
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("bob is working", || agents.prompts().len() == 1).await;
    let noticed = left_by_a_newer_build(&room, "Noticed", InboxWants::Answer, |_| {});
    room.arrived(&item(&room, "bob", &noticed));
    until("the notice reached the turn", || {
        item(&room, "bob", &noticed).state == InboxState::Noticed
    })
    .await;
    kept_its_fields(&room, &noticed, 1, "the mid-turn notice");
    gate.add_permits(100);
    until("bob is at rest", || {
        !lock(&room.session("bob").unwrap().turns).running
    })
    .await;

    // The work cannot be queued: the record of the two cannot be written.
    let unqueued = left_by_a_newer_build(&room, "Unqueued", InboxWants::Work, |_| {});
    let records = room.log.root().join("room.jsonl");
    let writable = std::fs::metadata(&records).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&records, read_only).unwrap();
    let refused = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": unqueued, "state": "taken"}]}),
    )
    .await;
    std::fs::set_permissions(&records, writable).unwrap();
    assert!(refused.is_err(), "{refused:?}");
    let let_go = item(&room, "bob", &unqueued);
    assert_eq!(let_go.state, InboxState::Taken, "it was claimed");
    assert_eq!(let_go.request, None, "and the claim was let go of");
    kept_its_fields(&room, &unqueued, 1, "letting go of a failed take");

    // The person stops the two while the work runs.
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(
        "inbox-newer-stopped",
        replies().gated(gate.clone()).consuming(),
    );
    let stopped = left_by_a_newer_build(&room, "Stopped", InboxWants::Work, |_| {});
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": stopped, "state": "taken"}]}),
    )
    .await
    .unwrap();
    until("the work runs", || agents.prompts().len() == 1).await;
    room.stop_exchange("ada", "bob").unwrap();
    until("the work is dropped", || {
        item(&room, "bob", &stopped).outcome == Some(InboxOutcome::Dropped)
    })
    .await;
    kept_its_fields(&room, &stopped, 1, "the person's Stop of its work");
    gate.add_permits(100);
}

/// One line of Bob's inbox as a newer build wrote it, in a state or with
/// an outcome no build has yet.
fn newer_line(id: &str, ts: i64, state: &str, extra: Value) -> Value {
    let mut line = json!({"id": id, "ts": ts, "from": "ada",
        "fromThread": {"kind": "dm", "key": "ada"}, "to": "bob", "title": format!("Title of {id}"),
        "body": format!("the words of {id}"), "wants": "answer", "state": state,
        "receipts": [{"state": state, "at": ts}], "senderTold": 0, "kind": "inbox_item"});
    if let (Some(line), Value::Object(extra)) = (line.as_object_mut(), extra) {
        line.extend(extra);
    }
    line
}

/// An item a newer build left in a state or with an outcome this build does
/// not know is listed as it is, where it stands, and is never rewritten:
/// not by a start, the clock long after it would have expired, a turn's
/// offer, reading it, or a sort, which is refused saying why; and a held
/// one's files are not swept.
#[tokio::test]
async fn an_item_in_a_state_or_with_an_outcome_this_build_does_not_know_is_listed_as_it_is_and_never_moved()
 {
    let log = scratch("inbox-newer-states");
    for (id, name) in [("ada", "Ada"), ("bob", "Bob")] {
        enrol(&log, &named(id, name, Some(Reach::Machine)));
    }
    let written = [
        newer_line("item-parked", 1, "parked", json!({})),
        newer_line(
            "item-forwarded",
            2,
            "closed",
            json!({"outcome": "forwarded"}),
        ),
        newer_line(
            "item-held-parked",
            3,
            "parked",
            json!({"held": true, "files": ["brief.txt"]}),
        ),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    let stream = log.root().join("inbox/bob.jsonl");
    std::fs::create_dir_all(stream.parent().unwrap()).unwrap();
    std::fs::write(&stream, &written).unwrap();
    let held = log.root().join("files-between/inbox-item-held-parked");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::write(held.join("brief.txt"), "the brief").unwrap();

    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(replies()));
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("ada", "bob").unwrap();
    room.recover_exchanges().await;
    room.inbox_due(now_ms() + 10 * expire_after()).await;
    room.inbox_told("bob", &ThreadId::dm("bob"), false);
    assert_eq!(room.inbox_unread("bob"), 0);

    let open = tool(&room, "bob", "read_inbox", json!({})).await.unwrap();
    let listed = &open["items"];
    assert_eq!(listed.as_array().unwrap().len(), 1, "{open}");
    assert_eq!(listed[0]["id"], short("item-parked"));
    assert_eq!(listed[0]["title"], "Title of item-parked");
    assert_eq!(listed[0]["from"], "Ada");
    assert_eq!(listed[0]["state"], "parked");
    assert_eq!(listed[0]["note"], NEWER_ITEM);
    let closed = tool(&room, "bob", "read_inbox", json!({"list": "closed"}))
        .await
        .unwrap();
    assert_eq!(closed["items"].as_array().unwrap().len(), 1, "{closed}");
    assert_eq!(closed["items"][0]["state"], "closed");
    assert_eq!(closed["items"][0]["outcome"], "forwarded");
    let sent = tool(&room, "ada", "read_inbox", json!({"list": "sent"}))
        .await
        .unwrap();
    let titles: Vec<&str> = sent["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "Title of item-held-parked",
            "Title of item-forwarded",
            "Title of item-parked"
        ],
        "newest first: {sent}"
    );
    assert_eq!(sent["items"][2]["to"], "Bob");

    let read = tool(&room, "bob", "read_inbox", json!({"ids": ["item-parked"]}))
        .await
        .unwrap();
    assert_eq!(read["items"][0]["message"], "the words of item-parked");
    assert_eq!(read["items"][0]["state"], "parked");
    for (id, entry) in [
        (
            "item-parked",
            json!({"id": "item-parked", "state": "closed", "outcome": "noted"}),
        ),
        (
            "item-forwarded",
            json!({"id": "item-forwarded", "answer": "Yes."}),
        ),
    ] {
        let refused = tool(&room, "bob", "update_inbox", json!({ "items": [entry] }))
            .await
            .unwrap_err();
        assert!(refused.contains(NEWER_ITEM), "{id}: {refused}");
    }
    let reply = tool(
        &room,
        "bob",
        "send",
        json!({"to": ["ada"], "title": "Re", "reply_to": "item-parked"}),
    )
    .await
    .unwrap_err();
    assert!(reply.contains(NEWER_ITEM), "{reply}");

    assert_eq!(
        std::fs::read_to_string(&stream).unwrap(),
        written,
        "nothing was written over them"
    );
    assert!(held.join("brief.txt").exists(), "its held files were kept");
}
