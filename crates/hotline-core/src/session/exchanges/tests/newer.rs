//! A desk opened by this build after a newer Hotline ran on it: what that
//! build wrote and this one does not read is left exactly as it was found.
//!
//! No build newer than this one exists yet, so the fixtures stand for any:
//! a field on a pair or a request this build does not know, a request in a
//! phase no build has yet, a folder of held files named as no build names
//! one, and an inbox item that wants what no build knows.

use super::*;
use crate::log::Log;

/// The newer build's requests, by id, as it wrote them into `ada~bob`.
fn newer_requests() -> Vec<serde_json::Value> {
    vec![
        // Work at its thread when the desk went down, with a field this
        // build does not know.
        json!({"id": "r-taken", "from": "ada", "to": "bob",
            "message": "Ship the notes\nFrom Ada, item 1a2b3c4d, wants work:\nDraft the release notes.",
            "intent": "handoff", "phase": "running", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false, "started": true,
            "result_consumed": false, "human_actions": [], "thread": "side-taken",
            "reply_thread": null, "returned": false, "items": ["item-taken"],
            "parcel": "p-1"}),
        // One waiting for a place among Bob's threads, the same.
        json!({"id": "r-queued", "from": "ada", "to": "bob",
            "message": "Tidy the changelog\nFrom Ada, item 2b3c4d5e, wants work:\nMerge the duplicates.",
            "intent": "handoff", "phase": "queued", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false, "started": false,
            "result_consumed": false, "human_actions": [], "thread": null,
            "reply_thread": null, "returned": false, "items": ["item-queued"],
            "parcel": "p-2"}),
        // A state no build has yet: this one cannot read it at all.
        json!({"id": "r-future", "from": "bob", "to": "ada", "message": "Later work",
            "intent": "handoff", "phase": "parked", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false, "started": false,
            "result_consumed": false, "human_actions": [], "thread": null,
            "reply_thread": null, "returned": false}),
    ]
}

/// A request this build wrote, which it reads and moves.
fn own_request() -> serde_json::Value {
    json!({"id": "r-own", "from": "ada", "to": "bob",
        "message": "Rename the flag\nFrom Ada, item 4d5e6f70, wants work:\nCall it quiet.",
        "intent": "handoff", "phase": "done", "reply": "Renamed.", "answer": "Renamed.",
        "failed": false, "reply_counted": true, "request_counted": true, "inline": false,
        "started": false, "result_consumed": true, "human_actions": [], "thread": "side-done",
        "reply_thread": null, "returned": false, "items": ["item-closed"]})
}

/// The pair as the newer build left it, this build's request among its own,
/// with a field of the newer build's on the pair.
fn ada_and_bob() -> serde_json::Value {
    let mut requests = newer_requests();
    requests.insert(2, own_request());
    json!({"id": "ada~bob", "a": "ada", "b": "bob", "exchanges": 4, "paused": false,
        "lane": "quiet", "requests": requests, "kind": "exchange_pair"})
}

/// A pair the person stopped, whose one request is the newer build's.
fn ada_and_cy() -> serde_json::Value {
    json!({"id": "ada~cy", "a": "ada", "b": "cy", "exchanges": 0, "paused": false,
        "stopped": true, "requests": [
            {"id": "r-dropped", "from": "ada", "to": "cy",
            "message": "Book the room\nFrom Ada, item 5e6f7081, wants work:\nFriday.",
            "intent": "handoff", "phase": "running", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false,
            "started": true, "result_consumed": false, "human_actions": [],
            "thread": "side-dropped", "reply_thread": null, "returned": false,
            "items": ["item-dropped"], "parcel": "p-3"}],
        "kind": "exchange_pair"})
}

/// The latest line of a pair on the room stream, as it is on disk.
fn pair_line(log: &Log, key: &str) -> serde_json::Value {
    log.load(&StreamId::Room)
        .into_iter()
        .find(|v| v["kind"] == "exchange_pair" && v["id"] == key)
        .expect("the pair is on the room stream")
}

/// The pair line without what this build moves on its own: its count, and
/// the person's stop, which the person's line clears.
fn kept_of(mut pair: serde_json::Value) -> serde_json::Value {
    let fields = pair.as_object_mut().unwrap();
    fields.remove("exchanges");
    fields.remove("stopped");
    pair
}

/// The desk as the newer build left it, before this one starts on it.
fn left_by_a_newer_build(name: &str) -> Log {
    let log = scratch(name);
    for id in ["ada", "bob", "cy"] {
        enrol(&log, &persona(id));
    }
    log.append(&StreamId::Room, &ada_and_bob()).unwrap();
    log.append(&StreamId::Room, &ada_and_cy()).unwrap();
    log
}

/// A start: the room opened on the log, and the pairs' workers woken, as
/// the desk does a moment after it opens.
fn start(log: Log) -> (Arc<Room>, Arc<Fake>) {
    let agents = answering();
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_queued_exchanges();
    (room, agents)
}

/// What a newer build wrote is all there and the same: the pair's own
/// field, and each request in its place, this build's among them read back
/// as it was written.
fn left_as_found(room: &Room) {
    assert_eq!(
        kept_of(pair_line(&room.log, "ada~bob")),
        kept_of(ada_and_bob())
    );
    assert_eq!(
        kept_of(pair_line(&room.log, "ada~cy")),
        kept_of(ada_and_cy())
    );
}

/// Nothing of the newer build's was moved here: no work of its was handed
/// to an agent, and nobody was told of a result or a failure of one.
fn nothing_moved(room: &Room, agents: &Fake) {
    for prompt in agents.prompts() {
        for words in ["release notes", "duplicates", "Later work", "Friday"] {
            assert!(!prompt.contains(words), "{words} was handed over: {prompt}");
        }
    }
    for persona in ["ada", "bob", "cy"] {
        for event in room.tape(persona) {
            let about = event["cause"]["requestId"]
                .as_str()
                .or_else(|| event["id"].as_str()?.strip_prefix("exchange-ended:"));
            assert!(
                !about.is_some_and(|id| id.starts_with("r-")),
                "{persona} was told of a newer build's request: {event}"
            );
        }
    }
}

/// The person's line to a teammate, its main conversation started first.
async fn say(room: &Arc<Room>, persona: &str, text: &str) {
    room.start(persona).await.unwrap();
    room.prompt(persona, text, None, None).await.unwrap();
}

/// The pair's worker has looked and found nothing of its own left to do.
async fn settled(room: &Room) {
    until("the pairs' workers retire", || {
        lock(&room.exchange_workers).is_empty()
    })
    .await;
}

/// A start, a person's line to each teammate, a start again (which compacts
/// the room stream first) and another line: a newer build's fields and
/// requests are all as it left them, and none of its work was queued, run,
/// failed, stopped or delivered.
#[tokio::test]
async fn a_start_and_the_persons_lines_leave_a_newer_builds_records_as_found() {
    let (room, agents) = start(left_by_a_newer_build("newer-left-alone"));
    settled(&room).await;
    left_as_found(&room);
    for persona in ["ada", "bob", "cy"] {
        say(&room, persona, "Morning.").await;
    }
    until("each teammate answered", || agents.prompts().len() == 3).await;
    // The person's line counts each pair afresh and lifts the stop, as it
    // always did, and touches nothing else.
    assert_eq!(pair_line(&room.log, "ada~bob")["exchanges"], 0);
    assert_eq!(pair_line(&room.log, "ada~cy").get("stopped"), None);
    left_as_found(&room);
    nothing_moved(&room, &agents);

    let log = room.log.clone();
    drop(room);
    let (room, agents) = start(log);
    settled(&room).await;
    let compacted = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(
        !compacted.contains(r#""exchanges":4"#),
        "the start compacted the pair's older lines away"
    );
    left_as_found(&room);
    say(&room, "bob", "Still there?").await;
    until("bob answered", || agents.prompts().len() == 1).await;
    left_as_found(&room);
    nothing_moved(&room, &agents);
}

/// One line of Bob's inbox, as the store writes it.
fn bobs_item(id: &str, wants: &str, state: &str, held: bool) -> String {
    let mut item = json!({"id": id, "ts": 1, "from": "ada",
        "fromThread": {"kind": "dm", "key": "ada"}, "to": "bob", "title": "The brief",
        "body": "", "wants": wants, "state": state, "receipts": [{"state": state, "at": 1}],
        "senderTold": 0, "kind": "inbox_item"});
    if held {
        item["held"] = json!(true);
        item["files"] = json!(["brief.txt"]);
    }
    item.to_string()
}

/// The start lets go of the held files this build can tell nothing will
/// carry: a request's that is done, an item's let in, an item's closed. It
/// keeps those a request or a held item still carries, and every folder it
/// cannot tell is spent: a newer build's request's, an item's it cannot
/// read, and one named as this build names none.
#[tokio::test]
async fn the_start_lets_go_only_of_held_files_this_build_knows_are_spent() {
    let log = scratch("newer-held-files");
    for id in ["ada", "bob"] {
        enrol(&log, &persona(id));
    }
    let mut pair = ada_and_bob();
    for request in pair["requests"].as_array_mut().unwrap() {
        request["files"] = json!(["notes.txt"]);
    }
    log.append(&StreamId::Room, &pair).unwrap();
    let inbox = [
        bobs_item("item-held", "nothing", "new", true),
        bobs_item("item-let-in", "nothing", "new", false),
        bobs_item("item-closed", "nothing", "closed", true),
        bobs_item("item-unread", "review", "new", true),
    ]
    .join("\n");
    std::fs::create_dir_all(log.root().join("inbox")).unwrap();
    std::fs::write(log.root().join("inbox/bob.jsonl"), inbox + "\n").unwrap();
    let held = |name: &str| log.root().join("files-between").join(name);
    let folders = [
        "r-own",
        "r-taken",
        "r-future",
        "inbox-item-held",
        "inbox-item-let-in",
        "inbox-item-closed",
        "inbox-item-unread",
        "parcel-p-1",
    ];
    for folder in folders {
        std::fs::create_dir_all(held(folder)).unwrap();
        std::fs::write(held(folder).join("notes.txt"), "notes").unwrap();
    }
    let _room = Room::with_agents(log.clone(), Arc::new(DeskKeys), answering());
    for spent in ["r-own", "inbox-item-let-in", "inbox-item-closed"] {
        assert!(!held(spent).exists(), "{spent} was let go of");
    }
    for kept in [
        "r-taken",
        "r-future",
        "inbox-item-held",
        "inbox-item-unread",
        "parcel-p-1",
    ] {
        assert!(held(kept).join("notes.txt").exists(), "{kept} was kept");
    }
}

/// A request in a state this build does not know, or with a field it does
/// not know, does not cost the pair its record: the pair reads, the request
/// is kept whole in its place, and a request this build adds goes after it.
#[test]
fn a_request_this_build_cannot_read_is_kept_whole_in_its_place() {
    let written = ada_and_bob();
    let mut pair: Pair = serde_json::from_value(written.clone()).expect("the pair reads");
    assert_eq!(
        pair.requests
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["r-own"],
        "only the request with nothing newer in it is this build's"
    );
    pair.requests
        .push(saved_request("r-new", Intent::Handoff, Phase::Queued));
    let mut kept = serde_json::to_value(&pair).unwrap();
    kept["kind"] = json!("exchange_pair");
    let mut expected = written["requests"].as_array().unwrap().clone();
    expected.push(serde_json::to_value(&pair.requests[1]).unwrap());
    assert_eq!(kept["requests"], json!(expected));
    let without_requests = |mut pair: serde_json::Value| {
        pair.as_object_mut().unwrap().remove("requests");
        pair
    };
    assert_eq!(without_requests(kept), without_requests(written));
    // An intent no build has yet is the same.
    let mut unknown = own_request();
    unknown["intent"] = json!("notice");
    let pair: Pair = serde_json::from_value(json!({"id": "ada~bob", "a": "ada", "b": "bob",
        "exchanges": 0, "paused": false, "requests": [unknown]}))
    .unwrap();
    assert!(pair.requests.is_empty());
    assert_eq!(serde_json::to_value(&pair).unwrap()["requests"][0], unknown);
}

/// What an older build wrote on a request and this one no longer reads does
/// not make the request a newer build's: it is read and moved, and written
/// back without those fields.
#[test]
fn a_request_with_only_an_older_builds_fields_is_this_builds() {
    let mut older = own_request();
    older["continues"] = json!("side:abc");
    older["crossing"] = json!([]);
    let pair: Pair = serde_json::from_value(json!({"id": "ada~bob", "a": "ada", "b": "bob",
        "exchanges": 0, "paused": false, "requests": [older]}))
    .unwrap();
    assert_eq!(pair.requests.len(), 1, "the request is this build's");
    assert_eq!(
        serde_json::to_value(&pair).unwrap()["requests"][0],
        own_request()
    );
}

/// A pair record this build cannot read at all is not replaced by a new
/// one: an item between the two, and the person's stop of them, are
/// refused, saying why, and the record stays as it was through a start and
/// the person's line.
#[tokio::test]
async fn a_pair_this_build_cannot_read_is_never_replaced() {
    let log = scratch("newer-unreadable");
    for id in ["ada", "bob"] {
        enrol(&log, &persona(id));
    }
    let written = json!({"id": "ada~bob", "a": "ada", "b": "bob", "exchanges": 2,
        "paused": "at the cap", "requests": [], "kind": "exchange_pair"});
    log.append(&StreamId::Room, &written).unwrap();
    let (room, _agents) = start(log);
    room.allow_sender("bob", "ada").unwrap();
    let refused = TeammateTools::new(&room, "ada")
        .call(
            "send",
            &json!({"to": ["bob"], "title": "Hello", "wants": "answer"}),
        )
        .await
        .unwrap_err();
    assert_eq!(refused, format!("{NEWER_PAIR} Nothing was sent."));
    assert_eq!(room.stop_exchange("ada", "bob").unwrap_err(), NEWER_PAIR);
    say(&room, "ada", "Morning.").await;
    assert_eq!(pair_line(&room.log, "ada~bob"), written);
    assert_eq!(
        crate::store::inbox::of(&room.log, "bob").len(),
        0,
        "nothing was sent"
    );
}
