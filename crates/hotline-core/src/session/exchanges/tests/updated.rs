//! A desk that ran 0.49.2 opened by this build: what was in flight between
//! two teammates when it was updated carries on as 0.49 would have carried
//! it, or is ended in Hotline's own words.
//!
//! The fixtures are pair records as 0.49.2 wrote them to the room stream
//! (`Record` there: its flags, no `items`, and `continues` on a follow-up).

use super::*;
use crate::contract::InboxNotice;
use crate::log::Log;

fn named(id: &str, name: &str) -> crate::contract::Persona {
    let mut teammate = persona(id);
    teammate.name = name.to_string();
    teammate
}

/// One request as 0.49.2 wrote it: every flag it always writes, and
/// `extra` for those it writes only when set.
fn written_by_0_49(
    id: &str,
    intent: &str,
    phase: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut request = json!({"id": id, "from": "ada", "to": "bob", "message": "",
        "intent": intent, "phase": phase, "reply": "", "failed": false,
        "reply_counted": false, "request_counted": true, "inline": false, "started": false,
        "result_consumed": false, "human_actions": [], "thread": null,
        "reply_thread": null, "returned": false});
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    request
}

/// The desk as 0.49.2 left it: Ada and Bob (and Cy), and the pairs given,
/// as the room stream holds them.
fn left_by_0_49(name: &str, pairs: &[serde_json::Value]) -> Log {
    let log = scratch(name);
    enrol(&log, &named("ada", "Ada"));
    enrol(&log, &named("bob", "Bob"));
    enrol(&log, &named("cy", "Cy"));
    for pair in pairs {
        log.append(&StreamId::Room, pair).unwrap();
    }
    log
}

fn pair_of(key: &str, b: &str, requests: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"id": key, "a": "ada", "b": b, "exchanges": 1, "paused": false,
        "requests": requests, "kind": "exchange_pair"})
}

/// The desk opens on the log as this build: the room is built, which settles
/// what the update left, and the pairs' workers are woken a moment later.
fn opened(log: Log) -> (Arc<Room>, Arc<Fake>) {
    let agents = answering();
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("cy", "ada").unwrap();
    room.recover_queued_exchanges();
    (room, agents)
}

/// A question Ada asked Bob before the update is ended, and Ada hears so
/// from Hotline, never in Bob's voice: one notice in her inbox, credited to
/// Hotline, that Bob never answered it.
#[tokio::test]
async fn an_ask_from_0_49_is_ended_by_hotlines_notice_not_in_the_colleagues_voice() {
    let asked = written_by_0_49(
        "r-ask",
        "ask",
        "running",
        json!({"message": "Which branch did you cut 0.49.2 from?"}),
    );
    let (room, agents) = opened(left_by_0_49(
        "updated-ask",
        &[pair_of("ada~bob", "bob", vec![asked])],
    ));
    until("Ada has Hotline's notice", || {
        crate::store::inbox::of(&room.log, "ada")
            .iter()
            .any(|item| item.notice.is_some())
    })
    .await;
    let inbox = crate::store::inbox::of(&room.log, "ada");
    assert_eq!(
        inbox.len(),
        1,
        "one notice and nothing in Bob's name: {inbox:?}"
    );
    let notice = &inbox[0];
    assert_eq!(notice.notice, Some(InboxNotice::Unanswered));
    assert_eq!(notice.from, "bob", "it is about Bob");
    assert_eq!(room.sender_of(notice), "Hotline");
    assert_eq!(
        notice.title,
        "Unanswered: Which branch did you cut 0.49.2 from?"
    );
    assert!(
        notice
            .body
            .starts_with("This is Hotline's notice, not Bob's words.")
            && notice.body.contains("Send it again with `send`"),
        "{}",
        notice.body
    );
    assert!(
        crate::store::inbox::sent_by(&room.log, "bob", ["ada", "bob", "cy"]).is_empty(),
        "Bob sent nothing"
    );
    let request = request(&room, "ada~bob", "r-ask");
    assert_eq!(request.phase(), Phase::Done);
    assert!(
        agents
            .prompts()
            .iter()
            .all(|prompt| !prompt.contains("Which branch")),
        "nothing asks Bob it again"
    );
}

/// A handoff Ada gave Bob before the update, waiting for its turn, runs
/// after it, and its result reaches Ada as it would have in 0.49: a turn of
/// her main conversation the person reads, not a quiet inbox item. So does
/// the result of one that finished while the desk was going down.
#[tokio::test]
async fn a_handoff_from_0_49_reports_its_result_out_loud() {
    let queued = written_by_0_49(
        "r-queued",
        "handoff",
        "queued",
        json!({"message": "Draft the 0.49.2 release notes"}),
    );
    let mut finished = written_by_0_49(
        "r-finished",
        "handoff",
        "reply",
        json!({"message": "Tag the release", "reply": "Tagged v0.49.2.",
            "answer": "Tagged v0.49.2.", "reply_counted": true, "thread": "side-tag"}),
    );
    finished["to"] = json!("cy");
    let (room, agents) = opened(left_by_0_49(
        "updated-handoff",
        &[
            pair_of("ada~bob", "bob", vec![queued]),
            pair_of("ada~cy", "cy", vec![finished]),
        ],
    ));
    until("both results are in Ada's conversation", || {
        ["r-queued", "r-finished"]
            .iter()
            .all(|id| result(&room, id).is_some())
    })
    .await;
    let queued = result(&room, "r-queued").unwrap();
    assert_eq!(queued["kind"], "delivery");
    assert_eq!(queued["cause"]["kind"], "peer");
    assert_eq!(queued["cause"]["status"], "done");
    let worked = queued["text"].as_str().unwrap().to_string();
    assert!(
        worked.starts_with("result-"),
        "Bob's work thread's words: {worked}"
    );
    assert_eq!(
        result(&room, "r-finished").unwrap()["text"],
        "Tagged v0.49.2."
    );
    until("Ada's turns hear both", || {
        let prompts = agents.prompts();
        prompts.iter().any(|p| p.contains(&worked))
            && prompts.iter().any(|p| p.contains("Tagged v0.49.2."))
    })
    .await;
    assert!(
        agents.prompts().iter().all(|p| !p.contains("quiet inbox")),
        "no quiet turn carries them: {:?}",
        agents.prompts()
    );
    assert!(
        crate::store::inbox::of(&room.log, "ada").is_empty(),
        "nothing waits in Ada's inbox"
    );
    until("the result was heard", || {
        request(&room, "ada~bob", "r-queued").heard()
    })
    .await;
}

/// A follow-up handoff Ada sent before the update to carry on the work
/// thread of an earlier one is handed into that thread, and keeps saying
/// which handoff it continues.
#[tokio::test]
async fn a_follow_up_from_0_49_keeps_its_thread() {
    let opened_it = written_by_0_49(
        "r-open",
        "handoff",
        "done",
        json!({"message": "Draft the release notes", "reply": "Drafted in notes.md.",
            "answer": "Drafted in notes.md.", "reply_counted": true,
            "result_consumed": true, "thread": "side-notes"}),
    );
    let follow_up = written_by_0_49(
        "r-follow",
        "handoff",
        "queued",
        json!({"message": "Add the Linux fixes to the notes", "continues": "r-open"}),
    );
    let (room, _) = opened(left_by_0_49(
        "updated-follow-up",
        &[pair_of("ada~bob", "bob", vec![opened_it, follow_up])],
    ));
    done(&room, "r-follow").await;
    assert_eq!(
        request(&room, "ada~bob", "r-follow").thread.as_deref(),
        Some("side-notes")
    );
    assert!(
        room.log
            .load(&StreamId::Side("side-notes".into()))
            .iter()
            .any(|line| line["id"] == "handoff:r-follow"),
        "the follow-up is heard in the thread it continues"
    );
    let kept = room
        .log
        .load(&StreamId::Room)
        .into_iter()
        .rfind(|line| line["kind"] == "exchange_pair" && line["id"] == "ada~bob")
        .unwrap();
    assert_eq!(kept["requests"][1]["continues"], "r-open");
}
