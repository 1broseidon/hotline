//! What the inbox keeps, and for how long (`inbox/tidy.rs`): a closed item
//! is removed once it has been kept as long as the desk keeps it, the
//! desk's opening writes each inbox again with each item's latest line, the
//! work an item was taken into leaves the record once it is over and its
//! items are gone, empty folders go, and `read_inbox` lists the newest open
//! items. Nothing a newer build wrote is touched.

use super::pick_up::reopened;
use super::*;

const DAY_MS: i64 = 24 * 3_600_000;

/// An item from Ada in Bob's inbox as this build keeps it: sent `ago` ms
/// before now and, when `closed` says how long ago, closed then, else taken
/// then, which never expires.
fn kept_item(room: &Room, title: &str, ago: i64, closed: Option<i64>) -> String {
    let ada = room.persona("ada").unwrap();
    let mut item = crate::session::inbox::item(
        &ada,
        &ThreadId::dm("ada"),
        "bob",
        title.into(),
        format!("the words of {title}"),
        InboxWants::Answer,
    );
    let now = now_ms();
    item.ts = now - ago;
    item.receipts.push(crate::contract::InboxReceipt {
        state: InboxState::New,
        at: now - ago,
        reason: None,
        outcome: None,
        into: None,
        newer: serde_json::Map::new(),
    });
    let Some(closed) = closed else {
        item.state = InboxState::Taken;
        item.receipts.push(crate::contract::InboxReceipt {
            state: InboxState::Taken,
            at: now - ago,
            reason: None,
            outcome: None,
            into: None,
            newer: serde_json::Map::new(),
        });
        room.log
            .append(&StreamId::Inbox("bob".into()), &kept::line(&item))
            .unwrap();
        return item.id;
    };
    {
        item.state = InboxState::Closed;
        item.outcome = Some(InboxOutcome::Noted);
        item.receipts.push(crate::contract::InboxReceipt {
            state: InboxState::Closed,
            at: now - closed,
            reason: None,
            outcome: Some(InboxOutcome::Noted),
            into: None,
            newer: serde_json::Map::new(),
        });
    }
    room.log
        .append(&StreamId::Inbox("bob".into()), &kept::line(&item))
        .unwrap();
    item.id
}

/// A line a newer build wrote in Bob's inbox, closed with an outcome no
/// build has yet, long ago, spaced as that build spaces it.
const NEWER: &str = r#"{"kind": "inbox_item", "id": "item-forwarded", "ts": 1, "from": "ada", "fromThread": {"kind": "dm", "key": "ada"}, "to": "bob", "title": "Forwarded", "body": "", "wants": "answer", "state": "closed", "outcome": "forwarded", "receipts": [{"state": "closed", "at": 1}], "senderTold": 0}"#;

fn bob_file(room: &Room) -> String {
    std::fs::read_to_string(room.log.root().join("inbox/bob.jsonl")).unwrap()
}

fn with_newer(room: &Room) {
    let path = room.log.root().join("inbox/bob.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    text.push_str(NEWER);
    text.push('\n');
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn ids(room: &Room) -> Vec<String> {
    kept::of(&room.log, "bob")
        .into_iter()
        .map(|item| item.title)
        .collect()
}

/// Closed items are kept 30 days unless the person says otherwise, then
/// removed by the clock; an open one stays however old, and one a newer
/// build saved stays, every byte of it, however long ago it closed.
#[tokio::test]
async fn a_closed_item_is_removed_once_kept_as_long_as_the_desk_keeps_it() {
    let (room, _) = room("inbox-keep");
    assert_eq!(super::super::tidy::keep_days(&room.log), 30);
    kept_item(&room, "Closed 31 days ago", 40 * DAY_MS, Some(31 * DAY_MS));
    kept_item(&room, "Closed 29 days ago", 40 * DAY_MS, Some(29 * DAY_MS));
    kept_item(&room, "Taken 40 days ago", 40 * DAY_MS, None);
    with_newer(&room);
    room.inbox_due(now_ms()).await;
    assert_eq!(ids(&room), ["Closed 29 days ago", "Taken 40 days ago"]);
    assert!(
        bob_file(&room).contains(&format!("{NEWER}\n")),
        "{}",
        bob_file(&room)
    );

    // The person keeps them a week.
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "inboxKeepDays", "value": 7}),
        )
        .unwrap();
    assert_eq!(super::super::tidy::keep_days(&room.log), 7);
    room.inbox_due(now_ms()).await;
    assert_eq!(ids(&room), ["Taken 40 days ago"]);
    assert!(bob_file(&room).contains(&format!("{NEWER}\n")));
}

/// The desk opening writes each inbox again with each item's latest line,
/// leaving a newer build's as it was written.
#[tokio::test]
async fn the_desk_opening_writes_each_inbox_again_with_each_items_latest_line() {
    let (room, _) = room("inbox-tidy-at-start");
    let id = send(&room, "ada", "bob", "Pick a name", json!({})).await;
    tool(&room, "bob", "read_inbox", json!({ "ids": [id] }))
        .await
        .unwrap();
    with_newer(&room);
    let before = bob_file(&room);
    assert!(before.lines().filter(|line| line.contains(&id)).count() > 1);
    let (room, _) = reopened(room);
    let after = bob_file(&room);
    assert_eq!(after.lines().filter(|line| line.contains(&id)).count(), 1);
    assert!(after.contains(&format!("{NEWER}\n")), "{after}");
    assert_eq!(item(&room, "bob", &id).state, InboxState::Seen);
}

/// Work an item was taken into leaves the two teammates' record once it is
/// over and the item has left the inbox, and the leases it held go once it
/// is over.
#[tokio::test]
async fn finished_work_leaves_the_record_once_its_item_is_gone() {
    let (room, _) = room("inbox-forget-work");
    let id = send(
        &room,
        "ada",
        "bob",
        "Fix the crane",
        json!({"wants": "work"}),
    )
    .await;
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "taken"}]}),
    )
    .await
    .unwrap();
    until("the work is done", || {
        item(&room, "bob", &id).state == InboxState::Closed
    })
    .await;
    let request = item(&room, "bob", &id).request.unwrap();
    let requests = |room: &Room| {
        pair(room).unwrap()["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|request| request["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    until("its result was heard", || {
        pair(&room).unwrap()["requests"][0]["result_consumed"] == true
    })
    .await;
    room.inbox_due(now_ms()).await;
    assert_eq!(
        requests(&room),
        std::slice::from_ref(&request),
        "its item is still kept"
    );
    room.inbox_tidy(now_ms(), true);
    assert!(!lock(&room.exchange_leases).contains_key(&request));
    room.inbox_tidy(now_ms() + 31 * DAY_MS, false);
    assert!(kept::of(&room.log, "bob").iter().all(|item| item.id != id));
    assert_eq!(requests(&room), [] as [String; 0]);
}

/// The desk opening lets go of the inbox's empty folders and keeps every
/// file.
#[tokio::test]
async fn the_desk_opening_lets_go_of_empty_folders_and_keeps_every_file() {
    let (room, _) = room("inbox-empty-folders");
    let root = room.log.root().to_path_buf();
    std::fs::create_dir_all(root.join("received/bob/ada")).unwrap();
    std::fs::create_dir_all(root.join("received/ada/bob")).unwrap();
    std::fs::write(root.join("received/ada/bob/q3.csv"), "a,b").unwrap();
    std::fs::create_dir_all(root.join("files-between")).unwrap();
    let (room, _) = reopened(room);
    let root = room.log.root();
    assert!(!root.join("received/bob").exists());
    assert!(!root.join("files-between").exists());
    assert!(root.join("received/ada/bob/q3.csv").exists());
}

/// `read_inbox` lists the newest 50 open items, and says how many older.
#[tokio::test]
async fn read_inbox_lists_the_newest_fifty_open_items() {
    let (room, _) = room("inbox-open-cap");
    for at in 0..55 {
        kept_item(&room, &format!("Item {at}"), (60 - at) * 60_000, None);
    }
    let listed = tool(&room, "bob", "read_inbox", json!({})).await.unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 50);
    assert_eq!(listed["items"][0]["title"], "Item 54");
    assert_eq!(listed["older"], 5);
}
