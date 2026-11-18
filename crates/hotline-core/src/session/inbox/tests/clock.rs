//! The inbox's clock (`inbox/clock.rs`): what is set aside is offered again
//! an hour after its first pick-up and then daily, by the items' own times;
//! what stays open three days expires, and its sender, not its receiver, is
//! told by Hotline's own notice, which no teammate can forge. Each test reads
//! the clock at a time it chooses (`Room::inbox_due`), so no test waits on
//! an hour.

use super::pick_up::{A_TURN, gated, idle, kinds_of, persons_turn, picked_up, reopened, sort};
use super::*;
use crate::contract::InboxNotice;
use crate::session::inbox::clock::{again_after_first, again_every, expire_after};
use std::sync::Arc;

/// An item `from` sent `to`, kept as `shape` leaves it, waking nobody: what
/// a desk that stopped left behind.
fn kept_item(
    room: &Room,
    from: &str,
    to: &str,
    title: &str,
    shape: impl FnOnce(&mut InboxItem),
) -> InboxItem {
    let sender = room.persona(from).unwrap();
    let mut kept = crate::session::inbox::item(
        &sender,
        &ThreadId::dm(from),
        to,
        title.into(),
        String::new(),
        InboxWants::Answer,
    );
    moved(&mut kept, InboxState::New, None, None, None);
    shape(&mut kept);
    room.keep_item(&kept).unwrap();
    kept
}

/// When an item arrived, as the clock reads it: its `new` receipt, which is
/// written a moment after the item is made, never its `ts`.
fn arrived(item: &InboxItem) -> i64 {
    item.receipts
        .iter()
        .rfind(|receipt| receipt.state == InboxState::New)
        .expect("an item that arrived has a new receipt")
        .at
}

/// Left for later, with why, after its turn offered it.
fn for_later(reason: &str) -> impl FnOnce(&mut InboxItem) {
    let reason = reason.to_string();
    move |item| {
        item.offered = Some(item.ts);
        let at = item.ts;
        moved(item, InboxState::Later, Some(reason), None, None);
        item.receipts.last_mut().unwrap().at = at;
    }
}

/// Hotline's notices in `persona_id`'s inbox.
fn notices(room: &Room, persona_id: &str) -> Vec<InboxItem> {
    items(room, persona_id)
        .into_iter()
        .filter(|item| item.notice.is_some())
        .collect()
}

/// Bob leaves Ada's item for later in the person's turn and is offered it
/// by the pick-up that follows: the first pick-up. Answers its id and when
/// it is due again.
async fn first_pick_up(
    room: &Arc<Room>,
    agents: &Fake,
    gate: &tokio::sync::Semaphore,
    session: &Session,
) -> (String, i64) {
    persons_turn(room, agents, "the person's work").await;
    let id = send(room, "ada", "bob", "Keyboards", json!({})).await;
    sort(
        room,
        json!([{"id": id, "state": "later", "reason": "after the research"}]),
    )
    .await;
    gate.add_permits(A_TURN);
    until("the first pick-up", || picked_up(agents).len() == 1).await;
    gate.add_permits(A_TURN);
    idle(session).await;
    let due = item(room, "bob", &id)
        .offer_again
        .expect("a pick-up sets when it is offered again");
    (id, due)
}

#[tokio::test]
async fn a_set_aside_item_is_offered_again_an_hour_after_its_first_pick_up_then_daily() {
    let (room, agents, gate, session) = gated("clock-again").await;
    let before = now_ms();
    let (id, due) = first_pick_up(&room, &agents, &gate, &session).await;
    assert!(
        due >= before + again_after_first() && due <= now_ms() + again_after_first(),
        "an hour after the first pick-up"
    );

    room.inbox_due(due - 1).await;
    assert!(!lock(&session.turns).running, "not before its time");
    room.inbox_due(due).await;
    assert!(lock(&session.turns).running, "offered again, at once");
    until("the timed offer", || picked_up(&agents).len() == 2).await;
    assert!(
        room.is_quiet(&ThreadId::dm("bob")),
        "as the same quiet turn"
    );
    assert!(
        picked_up(&agents)[1].contains(": \"after the research\""),
        "{}",
        picked_up(&agents)[1]
    );
    gate.add_permits(A_TURN);
    idle(&session).await;
    assert_eq!(
        lock(&session.pick_up).armed(),
        None,
        "its turn armed nothing"
    );

    let daily = item(&room, "bob", &id).offer_again.unwrap();
    assert_eq!(daily, due + again_every(), "then a day after that offer");
    room.inbox_due(daily - 1).await;
    assert!(!lock(&session.turns).running);
    room.inbox_due(daily).await;
    until("the daily offer", || picked_up(&agents).len() == 3).await;
    gate.add_permits(A_TURN);
    idle(&session).await;
    assert_eq!(
        item(&room, "bob", &id).offer_again,
        Some(daily + again_every())
    );
    assert_eq!(agents.told().len(), 4, "{:?}", agents.told());
}

/// Two items due by the same time are one turn, and one not yet due waits.
#[tokio::test]
async fn items_due_together_are_offered_in_one_turn_and_one_not_due_waits() {
    let (room, agents, gate, session) = gated("clock-together").await;
    let (first, due) = first_pick_up(&room, &agents, &gate, &session).await;
    let now = now_ms();
    let second = kept_item(&room, "ada", "bob", "Mice", |item| {
        for_later("after lunch")(item);
        item.offer_again = Some(now);
    });
    let third = kept_item(&room, "ada", "bob", "Desks", |item| {
        for_later("next week")(item);
        item.offer_again = Some(due + 1);
    });
    room.inbox_due(due).await;
    until("one offer", || picked_up(&agents).len() == 2).await;
    let line = &picked_up(&agents)[1];
    assert!(
        line.contains("\"Keyboards\"") && line.contains("\"Mice\""),
        "{line}"
    );
    assert!(!line.contains("\"Desks\""), "not due yet: {line}");
    gate.add_permits(A_TURN);
    idle(&session).await;
    assert_eq!(
        item(&room, "bob", &first).offer_again,
        Some(due + again_every())
    );
    assert_eq!(
        item(&room, "bob", &second.id).offer_again,
        Some(due + again_every())
    );
    assert_eq!(item(&room, "bob", &third.id).offer_again, Some(due + 1));
}

/// Busy when its time comes, the conversation is not offered the item by
/// the end of its turn, which never offers an item a second time: the
/// clock's next look once it is idle makes the offer, once.
#[tokio::test]
async fn a_timed_offer_that_finds_the_conversation_busy_is_made_by_the_clocks_next_look_once() {
    let (room, agents, gate, session) = gated("clock-busy").await;
    let (id, due) = first_pick_up(&room, &agents, &gate, &session).await;
    persons_turn(&room, &agents, "something else").await;
    room.inbox_due(due).await;
    assert_eq!(picked_up(&agents).len(), 1, "busy: nothing now");
    gate.add_permits(A_TURN);
    idle(&session).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(picked_up(&agents).len(), 1, "no pick-up at the turn's end");
    room.inbox_due(due + 1).await;
    until("the clock's offer", || picked_up(&agents).len() == 2).await;
    gate.add_permits(A_TURN);
    idle(&session).await;
    let next = item(&room, "bob", &id).offer_again.unwrap();
    assert_eq!(next, due + 1 + again_every(), "a day after that offer");
    room.inbox_due(due + 1).await;
    assert!(!lock(&session.turns).running, "nothing due now");
    assert_eq!(picked_up(&agents).len(), 2, "{:?}", agents.told());
}

/// Three days open, an item closes as expired with its receipt; its sender
/// gets one item back, Hotline's marked notice with the receiver's reason,
/// which wakes it quietly and tells it of the move; the receiver is not
/// woken; and a second look sends nothing more.
#[tokio::test]
async fn an_item_open_three_days_expires_and_its_sender_alone_gets_one_marked_notice() {
    let (room, agents) = room("clock-expire");
    let kept = kept_item(
        &room,
        "ada",
        "bob",
        "Keyboards",
        for_later("after the research"),
    );
    at_rest(&room, "ada").await;

    room.inbox_due(arrived(&kept) + expire_after() - 1).await;
    assert_eq!(item(&room, "bob", &kept.id).state, InboxState::Later);
    room.inbox_due(arrived(&kept) + expire_after()).await;
    let expired = item(&room, "bob", &kept.id);
    assert_eq!(expired.state, InboxState::Closed);
    assert_eq!(expired.outcome, Some(InboxOutcome::Expired));
    let receipt = expired.receipts.last().unwrap();
    assert_eq!(
        (receipt.state, receipt.outcome),
        (InboxState::Closed, Some(InboxOutcome::Expired))
    );

    let sent = notices(&room, "ada");
    assert_eq!(sent.len(), 1);
    let notice = &sent[0];
    assert_eq!(notice.notice, Some(InboxNotice::Expired));
    assert_eq!(notice.reply_to.as_deref(), Some(kept.id.as_str()));
    assert_eq!(notice.wants, InboxWants::Nothing);
    assert_eq!(notice.title, "Expired: Keyboards");
    assert!(
        notice
            .body
            .starts_with("This is Hotline's notice, not Bob's words.")
            && notice.body.contains("expired after 5 minutes")
            && notice
                .body
                .contains("Bob's last reason for leaving it for later: \"after the research\"."),
        "{}",
        notice.body
    );
    until("Ada is woken", || agents.told().len() == 1).await;
    let woken = &agents.told()[0];
    assert!(woken.contains(heard::QUIET_WAKE_LINE), "quietly: {woken}");
    assert!(
        woken.contains("\"Expired: Keyboards\" · from Hotline"),
        "Hotline's, not Bob's: {woken}"
    );
    assert!(
        woken.contains("\"Keyboards\" to Bob · closed, expired"),
        "told as any outcome is: {woken}"
    );
    until("Ada's turn ended", || {
        !lock(&room.session("ada").unwrap().turns).running
    })
    .await;
    assert_eq!(kinds_of(&room, "ada", "said-0"), ["thought"]);
    assert!(room.session("bob").is_err(), "the receiver is not woken");
    let listed = tool(&room, "ada", "read_inbox", json!({})).await.unwrap();
    assert_eq!(listed["items"][0]["from"], "Hotline", "{listed}");
    assert_eq!(listed["items"][0]["notice"], "expired", "{listed}");
    let sent_list = tool(&room, "ada", "read_inbox", json!({"list": "sent"}))
        .await
        .unwrap();
    assert_eq!(sent_list["items"][0]["outcome"], "expired", "{sent_list}");
    let bobs = tool(&room, "bob", "read_inbox", json!({"list": "sent"}))
        .await
        .unwrap();
    assert_eq!(bobs["items"], json!([]), "Bob sent no notice: {bobs}");

    room.inbox_due(kept.ts + 10 * expire_after()).await;
    assert_eq!(
        items(&room, "ada")
            .iter()
            .filter(
                |item| item.notice.is_some() && item.reply_to.as_deref() == Some(kept.id.as_str())
            )
            .count(),
        1,
        "exactly one notice"
    );
}

/// An item sent in a turn of the sender's main conversation that the person
/// read was something they asked for: when it expires, unread as it is,
/// Hotline's notice tells the sender to say in one line that it was never
/// picked up. One sent from a quiet turn leaves that to the sender, and both
/// expire by the same three days.
#[tokio::test]
async fn an_expired_item_the_person_asked_for_has_its_sender_tell_them_it_was_never_picked_up() {
    let (room, _) = room("clock-for-person");
    let main = ThreadId::dm("ada");
    let asked = {
        let _turn = room.open_posting(&main, None);
        send(&room, "ada", "bob", "Keyboards", json!({})).await
    };
    let unasked = {
        let _turn = room.open_posting(&main, None);
        room.set_quiet(&main, true);
        send(&room, "ada", "bob", "Mice", json!({})).await
    };
    let outside = send(&room, "ada", "bob", "Trackpads", json!({})).await;
    assert!(item(&room, "bob", &asked).for_person);
    assert!(!item(&room, "bob", &unasked).for_person, "a quiet turn's");
    assert!(!item(&room, "bob", &outside).for_person, "no turn's");

    let first = item(&room, "bob", &asked);
    room.inbox_due(arrived(&first) + expire_after() - 1).await;
    assert!(notices(&room, "ada").is_empty(), "three days, not sooner");
    room.inbox_due(now_ms() + expire_after()).await;
    for id in [&asked, &unasked, &outside] {
        assert_eq!(item(&room, "bob", id).outcome, Some(InboxOutcome::Expired));
    }
    let said = |id: &str| {
        notices(&room, "ada")
            .into_iter()
            .find(|notice| notice.reply_to.as_deref() == Some(id))
            .expect("a notice for each")
            .body
    };
    let told = said(&asked);
    assert!(
        told.starts_with("This is Hotline's notice, not Bob's words.")
            && told.contains(
                "Hotline gives you a turn they read, and you tell them then, in one line, that Bob never picked it up."
            ),
        "{told}"
    );
    for id in [&unasked, &outside] {
        let left = said(id);
        assert!(
            left.ends_with("Tell the person only if it is theirs to know.")
                && !left.contains("never picked it up"),
            "{left}"
        );
    }
    // The sort that reads the notice credits it to Hotline, not to Bob.
    let notice = notices(&room, "ada").remove(0);
    tool(
        &room,
        "ada",
        "update_inbox",
        json!({"items": [{"id": notice.id, "state": "closed", "outcome": "noted"}]}),
    )
    .await
    .unwrap();
    let sorted = room
        .tape("ada")
        .into_iter()
        .rfind(|event| event["kind"] == "inbox_sorted")
        .expect("the sort's line");
    assert_eq!(sorted["entries"][0]["notice"], "expired", "{sorted}");
}

/// A notice that expires closes as any item does, and tells nobody.
#[tokio::test]
async fn an_expired_notice_sends_no_notice_of_its_own() {
    let (room, _) = room("clock-no-chain");
    let kept = kept_item(
        &room,
        "ada",
        "bob",
        "Keyboards",
        for_later("after the research"),
    );
    room.inbox_due(arrived(&kept) + expire_after()).await;
    let notice = notices(&room, "ada").remove(0);
    room.inbox_due(arrived(&notice) + expire_after()).await;
    assert_eq!(
        item(&room, "ada", &notice.id).outcome,
        Some(InboxOutcome::Expired)
    );
    assert_eq!(items(&room, "bob").len(), 1, "no notice back to Bob");
    assert_eq!(notices(&room, "bob"), [] as [InboxItem; 0]);
}

/// Work taken from the inbox is being done: it never expires.
#[tokio::test]
async fn a_taken_item_never_expires() {
    let (room, _) = room("clock-taken");
    let kept = kept_item(
        &room,
        "ada",
        "bob",
        "Keyboards",
        for_later("after the research"),
    );
    sort(
        &room,
        json!([{"id": kept.id, "state": "taken", "in_thread": false}]),
    )
    .await;
    room.inbox_due(kept.ts + 10 * expire_after()).await;
    assert_eq!(item(&room, "bob", &kept.id).state, InboxState::Taken);
    assert_eq!(notices(&room, "ada"), [] as [InboxItem; 0]);
}

/// `expired` and the notice's mark are Hotline's alone: `update_inbox`
/// refuses the outcome, `send` and an answer never carry the mark, a title
/// that says "Expired" is still its sender's, and a notice cannot be
/// answered as if it were a colleague's message.
#[tokio::test]
async fn a_teammate_can_neither_set_expired_nor_forge_hotlines_notice() {
    let (room, _) = room("clock-forge");
    let id = send(&room, "ada", "bob", "Keyboards", json!({})).await;
    let refused = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "closed", "outcome": "expired"}]}),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("done, noted or dropped"), "{refused}");
    assert!(item(&room, "bob", &id).state.unread());

    let forged = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "title": "Expired: Keyboards", "notice": "expired"}),
    )
    .await;
    if let Ok(sent) = forged {
        let forged = sent["sent"][0]["id"].as_str().unwrap();
        assert_eq!(item(&room, "bob", forged).notice, None);
    }
    sort(
        &room,
        json!([{"id": id, "answer": "Here.", "notice": "expired"}]),
    )
    .await;
    assert!(
        items(&room, "ada").iter().all(|item| item.notice.is_none()),
        "an answer is never a notice"
    );
    assert!(
        items(&room, "bob").iter().all(|item| item.notice.is_none()),
        "a send is never a notice"
    );
    let listed = tool(&room, "bob", "read_inbox", json!({})).await.unwrap();
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["from"] == "Ada" && row.get("notice").is_none()),
        "{listed}"
    );

    // Overdue now, so its notice, sent now, is not.
    kept_item(&room, "bob", "ada", "Mice", |item| {
        item.ts = now_ms() - expire_after();
        item.receipts[0].at = item.ts;
        for_later("later")(item);
    });
    room.inbox_due(now_ms()).await;
    let notice = notices(&room, "bob")
        .into_iter()
        .find(|notice| notice.title == "Expired: Mice")
        .expect("Bob is told Mice expired");
    let answered = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": notice.id, "answer": "Sorry."}]}),
    )
    .await
    .unwrap_err();
    assert!(answered.contains("Hotline's notice"), "{answered}");
}

/// A desk that opens with one item overdue to expire and one overdue to be
/// offered again closes the first once, sends one notice, and offers the
/// second once, at start; the clock's next look adds nothing.
#[tokio::test]
async fn a_desk_that_opens_with_overdue_items_makes_one_expiry_and_one_offer_not_a_burst() {
    let (room, _) = room("clock-restart");
    let now = now_ms();
    let stale = kept_item(&room, "ada", "bob", "Old", |item| {
        item.ts = now - expire_after() - 1_000;
        item.receipts[0].at = item.ts;
        for_later("someday")(item);
    });
    let due = kept_item(&room, "ada", "bob", "Keyboards", |item| {
        for_later("after the research")(item);
        item.offer_again = Some(now - 1_000);
    });
    let (room, agents) = reopened(room);
    room.recover_exchanges().await;
    assert_eq!(
        item(&room, "bob", &stale.id).outcome,
        Some(InboxOutcome::Expired),
        "closed before anything is offered"
    );
    until("Bob is offered the one still open", || {
        picked_up(&agents).len() == 1
    })
    .await;
    let line = &picked_up(&agents)[0];
    assert!(
        line.contains("\"Keyboards\"") && !line.contains("\"Old\""),
        "{line}"
    );
    let bob = room.session("bob").unwrap();
    idle(&bob).await;
    room.inbox_due(now_ms()).await;
    room.recover_exchanges().await;
    tokio::time::sleep(heard::gather_delay() * 4).await;
    assert_eq!(picked_up(&agents).len(), 1, "{:?}", agents.told());
    assert_eq!(notices(&room, "ada").len(), 1);
    assert!(item(&room, "bob", &due.id).offer_again.unwrap() > now_ms());
    assert!(
        agents
            .told()
            .iter()
            .any(|told| told.contains("\"Expired: Old\" · from Hotline")),
        "Ada is woken by her notice: {:?}",
        agents.told()
    );
}
