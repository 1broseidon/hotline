//! A desk opened by this build after a newer Hotline ran on it: what that
//! build wrote and this one does not read is left exactly as it was found.
//!
//! The fixtures are what the inbox prototype (0.50) writes: a pair's
//! requests carry the inbox `items` they work, a pair the person stopped
//! carries `stopped`, the items themselves are on `inbox/<id>.jsonl`, a held
//! item's files wait in `files-between/inbox-<item id>/`, and tapes carry
//! `inbox_sent` and `inbox_sorted` lines. One request is in a phase no build
//! has yet, standing for any state this one does not know.

use super::*;
use crate::log::Log;

/// The newer build's requests, by id, as it wrote them into `ada~bob`.
fn newer_requests() -> Vec<serde_json::Value> {
    vec![
        // An item Bob took, at work in its thread when the desk went down.
        json!({"id": "r-taken", "from": "ada", "to": "bob",
            "message": "Ship the notes\nFrom Ada, item 1a2b3c4d, wants work:\nDraft the release notes.",
            "intent": "handoff", "phase": "running", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false, "started": true,
            "result_consumed": false, "human_actions": [], "thread": "side-taken",
            "reply_thread": null, "returned": false, "items": ["item-taken"]}),
        // One taken and still waiting for a place among Bob's threads.
        json!({"id": "r-queued", "from": "ada", "to": "bob",
            "message": "Tidy the changelog\nFrom Ada, item 2b3c4d5e, wants work:\nMerge the duplicates.",
            "intent": "handoff", "phase": "queued", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false, "started": false,
            "result_consumed": false, "human_actions": [], "thread": null,
            "reply_thread": null, "returned": false, "items": ["item-queued", "item-later"]}),
        // One whose result is not yet in Ada's inbox.
        json!({"id": "r-reply", "from": "ada", "to": "bob",
            "message": "Check the build\nFrom Ada, item 3c4d5e6f, wants work:\nIt failed overnight.",
            "intent": "handoff", "phase": "reply", "reply": "The build is green again.",
            "answer": "The build is green again.", "failed": false, "reply_counted": false,
            "request_counted": true, "inline": false, "started": false, "result_consumed": false,
            "human_actions": [], "thread": "side-reply", "reply_thread": null, "returned": false,
            "items": ["item-answered"]}),
        // One whose item closed as done.
        json!({"id": "r-done", "from": "ada", "to": "bob",
            "message": "Rename the flag\nFrom Ada, item 4d5e6f70, wants work:\nCall it quiet.",
            "intent": "handoff", "phase": "done", "reply": "Renamed.", "answer": "Renamed.",
            "failed": false, "reply_counted": true, "request_counted": true, "inline": false,
            "started": false, "result_consumed": true, "human_actions": [], "thread": "side-done",
            "reply_thread": null, "returned": false, "items": ["item-closed"]}),
        // A state no build has yet: this one cannot read it at all.
        json!({"id": "r-future", "from": "bob", "to": "ada", "message": "Later work",
            "intent": "handoff", "phase": "parked", "reply": "", "failed": false,
            "reply_counted": false, "request_counted": true, "inline": false, "started": false,
            "result_consumed": false, "human_actions": [], "thread": null,
            "reply_thread": null, "returned": false}),
    ]
}

/// A request from before the inbox, which this build reads and moves.
fn older_request() -> serde_json::Value {
    json!({"id": "r-old", "from": "ada", "to": "bob", "message": "Which port?",
        "intent": "ask", "phase": "done", "reply": "8080.", "answer": "8080.", "failed": false,
        "reply_counted": true, "request_counted": true, "inline": false, "started": false,
        "result_consumed": true, "human_actions": [], "thread": null, "reply_thread": null,
        "returned": false})
}

/// The pair as the newer build left it, the older request among its own.
fn ada_and_bob() -> serde_json::Value {
    let mut requests = newer_requests();
    requests.insert(2, older_request());
    json!({"id": "ada~bob", "a": "ada", "b": "bob", "exchanges": 4, "paused": false,
        "requests": requests, "kind": "exchange_pair"})
}

/// A pair the person stopped on the newer build, with the request the stop
/// dropped.
fn ada_and_cy() -> serde_json::Value {
    json!({"id": "ada~cy", "a": "ada", "b": "cy", "exchanges": 0, "paused": false,
        "stopped": true, "requests": [
            {"id": "r-dropped", "from": "ada", "to": "cy",
            "message": "Book the room\nFrom Ada, item 5e6f7081, wants work:\nFriday.",
            "intent": "handoff", "phase": "stopped", "reply": "Stopped by the person.",
            "failed": true, "reply_counted": false, "request_counted": true, "inline": false,
            "started": true, "result_consumed": false, "human_actions": [],
            "thread": "side-dropped", "reply_thread": null, "returned": true,
            "released": true, "items": ["item-dropped"]}],
        "kind": "exchange_pair"})
}

/// Bob's inbox: an open item, one taken, one for later, one closed, and one
/// held for the person's card with a file.
const BOBS_INBOX: &str = concat!(
    r#"{"id":"item-new","ts":1,"from":"ada","fromThread":{"kind":"dm","key":"ada"},"to":"bob","title":"Look at the logs","body":"","wants":"answer","state":"new","receipts":[{"state":"new","at":1}],"senderTold":1,"kind":"inbox_item"}"#,
    "\n",
    r#"{"id":"item-taken","ts":2,"from":"ada","fromThread":{"kind":"dm","key":"ada"},"to":"bob","title":"Ship the notes","body":"Draft the release notes.","wants":"work","state":"taken","receipts":[{"state":"new","at":2},{"state":"taken","at":3}],"work":{"kind":"side","key":"side-taken"},"request":"r-taken","senderTold":2,"kind":"inbox_item"}"#,
    "\n",
    r#"{"id":"item-later","ts":4,"from":"ada","fromThread":{"kind":"dm","key":"ada"},"to":"bob","title":"Tidy the docs","body":"","wants":"work","state":"later","reason":"after the release","receipts":[{"state":"new","at":4},{"state":"later","at":5,"reason":"after the release"}],"senderTold":2,"kind":"inbox_item"}"#,
    "\n",
    r#"{"id":"item-closed","ts":6,"from":"ada","fromThread":{"kind":"dm","key":"ada"},"to":"bob","title":"Rename the flag","body":"Call it quiet.","wants":"work","state":"closed","outcome":"done","receipts":[{"state":"new","at":6},{"state":"taken","at":7},{"state":"closed","at":8,"outcome":"done"}],"work":{"kind":"side","key":"side-done"},"request":"r-done","senderTold":3,"kind":"inbox_item"}"#,
    "\n",
    r#"{"id":"item-held","ts":9,"from":"cy","fromThread":{"kind":"dm","key":"cy"},"to":"bob","title":"The brief","body":"","wants":"nothing","state":"new","receipts":[{"state":"new","at":9}],"held":true,"files":["brief.txt"],"senderTold":0,"kind":"inbox_item"}"#,
    "\n",
);

/// Lines the newer build left on the tapes, of kinds this build draws as
/// nothing.
fn inbox_sent() -> serde_json::Value {
    json!({"kind": "inbox_sent", "id": "inbox-sent:item-taken", "ts": 2, "item": "item-taken",
        "to": "bob", "title": "Ship the notes"})
}
fn inbox_sorted() -> serde_json::Value {
    json!({"kind": "inbox_sorted", "id": "inbox-sorted:1", "ts": 3,
        "entries": [{"item": "item-taken", "title": "Ship the notes", "from": "ada", "state": "taken"},
            {"item": "item-later", "title": "Tidy the docs", "from": "ada", "state": "later",
            "reason": "after the release"}],
        "open": 3, "unread": 1})
}

fn held_file(log: &Log) -> std::path::PathBuf {
    log.root()
        .join("files-between")
        .join("inbox-item-held")
        .join("brief.txt")
}

/// The desk as the newer build left it, before this one starts on it.
fn left_by_a_newer_build(name: &str) -> Log {
    let log = scratch(name);
    for id in ["ada", "bob", "cy"] {
        enrol(&log, &persona(id));
    }
    log.append(&StreamId::Room, &ada_and_bob()).unwrap();
    log.append(&StreamId::Room, &ada_and_cy()).unwrap();
    log.append(&StreamId::Tape("ada".into()), &inbox_sent())
        .unwrap();
    log.append(&StreamId::Tape("bob".into()), &inbox_sorted())
        .unwrap();
    std::fs::create_dir_all(log.root().join("inbox")).unwrap();
    std::fs::write(log.root().join("inbox/bob.jsonl"), BOBS_INBOX).unwrap();
    std::fs::create_dir_all(held_file(&log).parent().unwrap()).unwrap();
    std::fs::write(held_file(&log), "the brief").unwrap();
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

/// The latest line of a pair on the room stream, as it is on disk.
fn pair_line(log: &Log, key: &str) -> serde_json::Value {
    log.load(&StreamId::Room)
        .into_iter()
        .find(|v| v["kind"] == "exchange_pair" && v["id"] == key)
        .expect("the pair is on the room stream")
}

/// What a newer build wrote is all there and the same: the pair's own
/// field, each of its requests in its place, the older request among them
/// read back as it was written, the inbox, the held file and the tapes'
/// lines.
fn left_as_found(room: &Room) {
    let log = &room.log;
    let pair = pair_line(log, "ada~bob");
    assert_eq!(pair["requests"], ada_and_bob()["requests"]);
    let stopped = pair_line(log, "ada~cy");
    assert_eq!(stopped["stopped"], true);
    assert_eq!(stopped["requests"], ada_and_cy()["requests"]);
    assert_eq!(
        std::fs::read_to_string(log.root().join("inbox/bob.jsonl")).unwrap(),
        BOBS_INBOX
    );
    assert_eq!(
        std::fs::read_to_string(held_file(log)).unwrap(),
        "the brief"
    );
    assert!(room.tape("ada").contains(&inbox_sent()));
    assert!(room.tape("bob").contains(&inbox_sorted()));
}

/// Nothing of the newer build's was moved here: no work of its was handed
/// to an agent, and nobody was told of a result or a failure of one.
fn nothing_moved(room: &Room, agents: &Fake) {
    for prompt in agents.prompts() {
        for words in [
            "release notes",
            "duplicates",
            "overnight",
            "Call it quiet",
            "Friday",
        ] {
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
/// the room stream first) and another line: a newer build's fields,
/// requests, inbox, held files and tape lines are all as it left them, and
/// none of its work was queued, run, failed, stopped or delivered.
#[tokio::test]
async fn a_start_and_the_persons_lines_leave_a_newer_builds_records_as_found() {
    let (room, agents) = start(left_by_a_newer_build("newer-left-alone"));
    settled(&room).await;
    left_as_found(&room);
    for persona in ["ada", "bob", "cy"] {
        say(&room, persona, "Morning.").await;
    }
    until("each teammate answered", || agents.prompts().len() == 3).await;
    // The person's line counts each pair afresh, as it always did, and
    // touches nothing else.
    assert_eq!(pair_line(&room.log, "ada~bob")["exchanges"], 0);
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

/// The same requests with nothing a newer build added are this build's,
/// and move as they always did: the one cut off mid-work fails, the one
/// waiting is handed over and answered, and a result is delivered. Of the
/// files this build held, a folder nothing carries is let go of and one a
/// request still carries is kept.
#[tokio::test]
async fn the_same_records_without_a_newer_builds_fields_move_as_before() {
    let log = scratch("newer-stripped");
    for id in ["ada", "bob"] {
        enrol(&log, &persona(id));
    }
    let mut pair = ada_and_bob();
    let requests = pair["requests"].as_array_mut().unwrap();
    requests.retain(|r| r["id"] != "r-future");
    for request in requests.iter_mut() {
        request.as_object_mut().unwrap().remove("items");
    }
    requests.iter_mut().find(|r| r["id"] == "r-reply").unwrap()["files"] = json!(["notes.txt"]);
    log.append(&StreamId::Room, &pair).unwrap();
    let held = |name: &str| log.root().join("files-between").join(name);
    for folder in ["held-0f3c", "r-old", "r-reply"] {
        std::fs::create_dir_all(held(folder)).unwrap();
        std::fs::write(held(folder).join("notes.txt"), "notes").unwrap();
    }
    let agents = answering();
    let room = Room::with_agents(log.clone(), Arc::new(DeskKeys), agents.clone());
    assert!(
        !held("held-0f3c").exists(),
        "a send cut off before it queued"
    );
    assert!(!held("r-old").exists(), "a request that is done");
    assert!(held("r-reply").exists(), "a result still carries its files");
    room.allow_sender("bob", "ada").unwrap();
    room.recover_queued_exchanges();
    for id in ["r-taken", "r-queued", "r-reply"] {
        done(&room, id).await;
    }
    let delivered = |id: &str| {
        room.tape("ada")
            .into_iter()
            .find(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id)
            .unwrap_or_else(|| panic!("{id} was not delivered"))
    };
    assert!(
        delivered("r-taken")["text"]
            .as_str()
            .unwrap()
            .contains("Hotline restarted before the result was saved")
    );
    assert!(
        delivered("r-reply")["text"]
            .as_str()
            .unwrap()
            .contains("The build is green again.")
    );
    delivered("r-queued");
    assert!(
        agents
            .prompts()
            .iter()
            .any(|prompt| prompt.contains("Merge the duplicates.")),
        "the waiting handoff was handed to Bob"
    );
}

/// A request in a state this build does not know does not cost the pair
/// its record: the pair reads, the request is kept whole in its place, and
/// a request this build adds goes after it.
#[test]
fn a_request_in_a_state_this_build_does_not_know_is_kept_whole_in_its_place() {
    let written = ada_and_bob();
    let mut pair: Pair = serde_json::from_value(written.clone()).expect("the pair reads");
    assert_eq!(
        pair.requests
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["r-old"],
        "only the request with nothing newer in it is this build's"
    );
    pair.requests
        .push(saved_request("r-new", Intent::Ask, Phase::Queued));
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
    let mut unknown = older_request();
    unknown["intent"] = json!("notice");
    let pair: Pair = serde_json::from_value(json!({"id": "ada~bob", "a": "ada", "b": "bob",
        "exchanges": 0, "paused": false, "requests": [unknown]}))
    .unwrap();
    assert!(pair.requests.is_empty());
    assert_eq!(serde_json::to_value(&pair).unwrap()["requests"][0], unknown);
}

/// A pair record this build cannot read at all is not replaced by a new
/// one: a message between the two is refused, saying why, and the record
/// stays as it was through a start and the person's line.
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
            "message_teammate",
            &json!({"to": "bob", "message": "Hello", "intent": "ask"}),
        )
        .await
        .unwrap_err();
    assert_eq!(refused, NEWER_PAIR);
    say(&room, "ada", "Morning.").await;
    assert_eq!(pair_line(&room.log, "ada~bob"), written);
}

/// A teammate whose conversation from a newer desk names one of its tools
/// is told plainly why it is missing here.
#[tokio::test]
async fn a_call_to_a_newer_builds_tool_says_this_desk_is_older() {
    let (room, _agents) = setup("newer-tool");
    for tool in ["send", "read_inbox", "update_inbox"] {
        let refused = TeammateTools::new(&room, "ada")
            .call(tool, &json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            refused,
            format!(
                "This desk's Hotline is older than the one that had `{tool}`, so it has no such tool."
            )
        );
    }
    let refused = TeammateTools::new(&room, "ada")
        .call("summon", &json!({}))
        .await
        .unwrap_err();
    assert_eq!(refused, "This room has no tool called 'summon'.");
}
