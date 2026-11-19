//! `activity` and `outcome` through the real core, on both drivers, read off
//! the wire and off the disk.
//!
//! The scripted desk is `post_to_chat.rs`'s: a line `CALL <tool> <json>` is
//! one call to one of Hotline's own tools, and the turn's reply says what
//! each call answered. What the teammate says it is on and what its turn
//! left behind are lines on the thread it said them in, sent to a watching
//! window as they land and read back the same after a restart; an open
//! outcome is closed by its `id`, from the thread or from another of the
//! teammate's threads, and rewritten where it stands. An open outcome is
//! also one of the teammate's open loops, with its `review_by` and the job
//! it tracks.

use super::post_to_chat::{Desk, Driver, open, says};
use hotline_core::contract::{LoopSource, LoopState};
use hotline_core::log::{Log, StreamId};
use hotline_store::loops as kept;
use serde_json::{Value, json};

/// The declarations on a tape, as a fresh read of the disk has them: what a
/// window that reloads is sent.
fn declared_on_disk(desk: &Desk, persona_id: &str) -> Vec<Value> {
    Log::open(desk.root.path())
        .load(&StreamId::Tape(persona_id.to_string()))
        .into_iter()
        .filter(|event| matches!(event["kind"].as_str(), Some("activity" | "outcome")))
        .collect()
}

/// A turn says what it is on twice, once in a kind the desk has no mark
/// for, and what it left behind: a message sent, a server still up, and a
/// failure. Each call returns at once with a word, each is a line on the
/// tape in the order said and sent on the wire as it lands, and the same
/// lines are there when the tape is read again from disk. The calls
/// themselves are steps whose glance names them, so a window can head the
/// path with them rather than list them (on Hotline Agent, whose steps the
/// desk writes; the fixture child reports none).
async fn a_turn_says_what_it_is_on_and_what_it_left(driver: Driver) {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", driver, false).await;
    let too_long = "a".repeat(81);
    let script = [
        r#"CALL activity {"type":"reading","message":"Reading the lease"}"#.to_string(),
        r#"CALL activity {"type":"juggling"}"#.to_string(),
        r#"CALL outcome {"type":"sent","message":"Sent Huck a message"}"#.to_string(),
        r#"CALL outcome {"type":"open","message":"Web server running","id":"srv"}"#.to_string(),
        r#"CALL outcome {"type":"sent","message":"Could not reach the school","state":"failed"}"#
            .to_string(),
        format!(r#"CALL activity {{"type":"making","message":"{too_long}"}}"#),
    ]
    .join("\n");
    let sent = desk
        .client
        .call("session.prompt", json!({"personaId": ada, "text": script}))
        .await;
    assert_eq!(sent["ok"], true, "{sent}");

    let reading = desk
        .event(tape, |event| {
            event["kind"] == "activity" && event["type"] == "reading"
        })
        .await;
    assert_eq!(reading["message"], "Reading the lease");
    let results = desk.results(tape).await;
    says(&results, 0, "Noted.");
    says(&results, 1, "shown plainly");
    says(&results, 2, "Noted.");
    says(&results, 3, "id 'srv'");
    says(&results, 4, "Noted.");
    says(&results, 5, "at most 80 characters");

    let lines = declared_on_disk(&desk, &ada);
    let said: Vec<(Value, Value, Value)> = lines
        .iter()
        .map(|line| {
            (
                line["kind"].clone(),
                line["type"].clone(),
                line["message"].clone(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            (
                json!("activity"),
                json!("reading"),
                json!("Reading the lease")
            ),
            (json!("activity"), json!("juggling"), Value::Null),
            (
                json!("outcome"),
                json!("sent"),
                json!("Sent Huck a message")
            ),
            (json!("outcome"), json!("open"), json!("Web server running")),
            (
                json!("outcome"),
                json!("sent"),
                json!("Could not reach the school")
            ),
        ],
        "the refused activity landed nowhere: {lines:#?}"
    );
    assert_eq!(lines[3]["key"], "srv");
    assert!(lines[3].get("closedAt").is_none());
    assert_eq!(lines[4]["state"], "failed");
    assert!(
        lines[2].get("state").is_none(),
        "done is the default and unsaid"
    );

    // The fixture child reports no steps of its own; Hotline Agent's are the desk's.
    if matches!(driver, Driver::Acp) {
        return;
    }
    let steps: Vec<Value> = desk
        .log
        .load(&StreamId::Tape(ada.clone()))
        .into_iter()
        .filter(|event| event["kind"] == "tool")
        .map(|event| event["said"]["key"].clone())
        .collect();
    assert!(
        steps.iter().filter(|key| *key == "activity").count() >= 3
            && steps.iter().any(|key| *key == "outcome"),
        "each call is a step its glance names: {steps:?}"
    );
}

/// An open outcome closes by its `id`: again in a later turn it is updated
/// where it stands, `ended` takes it off the line with its first time kept,
/// and an `id` with nothing open is refused. A work thread of the
/// teammate's closes one its main conversation opened, as what it became.
async fn an_open_outcome_closes_by_its_id(driver: Driver) {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", driver, false).await;
    let prompt = |text: &str| json!({"personaId": ada, "text": text});
    let opened = [
        r#"CALL outcome {"type":"open","message":"Web server running","id":"srv"}"#,
        r#"CALL outcome {"type":"open","message":"Waiting on quotes: 0 of 4","id":"quotes"}"#,
    ]
    .join("\n");
    assert_eq!(
        desk.client.call("session.prompt", prompt(&opened)).await["ok"],
        true
    );
    desk.results(tape).await;
    let first = declared_on_disk(&desk, &ada);
    let server_ts = first[0]["ts"].clone();

    let later = [
        r#"CALL outcome {"type":"open","message":"Web server running on 8000","id":"srv"}"#,
        r#"CALL outcome {"type":"open","message":"Web server stopped","id":"srv","ended":true}"#,
        r#"CALL outcome {"type":"open","message":"Nothing","id":"nope","ended":true}"#,
    ]
    .join("\n");
    assert_eq!(
        desk.client.call("session.prompt", prompt(&later)).await["ok"],
        true
    );
    let results = desk.results(tape).await;
    says(&results, 0, "Updated.");
    says(&results, 1, "Ended.");
    says(&results, 2, "no open outcome called 'nope'");

    let thread = desk
        .client
        .call(
            "thread.open",
            json!({"personaId": ada, "text": r#"CALL outcome {"type":"made","message":"Three quotes in","id":"quotes"}"#}),
        )
        .await;
    assert_eq!(thread["ok"], true, "{thread}");
    let side = desk
        .client
        .subscribe(json!({"threadId": thread["result"]["thread"]}))
        .await;
    says(&desk.results(side).await, 0, "Closed.");

    let lines = declared_on_disk(&desk, &ada);
    assert_eq!(
        lines.len(),
        2,
        "closing rewrote the lines, it added none: {lines:#?}"
    );
    let server = &lines[0];
    assert_eq!(server["key"], "srv");
    assert_eq!(server["type"], "open");
    assert_eq!(server["message"], "Web server stopped");
    assert_eq!(
        server["ts"], server_ts,
        "it keeps its place under the turn that opened it"
    );
    assert!(server["closedAt"].is_i64(), "{server}");
    let quotes = &lines[1];
    assert_eq!(quotes["type"], "made");
    assert_eq!(quotes["message"], "Three quotes in");
    assert!(quotes["closedAt"].is_i64(), "{quotes}");
}

/// An open outcome is mirrored into the teammate's open loops through the
/// real tool: `review_by` becomes its review date, `job_id` binds it to a
/// live job of the teammate's, and ending that job closes it. A `job_id`
/// that names no live job of the teammate's is refused, and nothing lands.
async fn an_open_outcome_is_an_open_loop_tracking_its_job(driver: Driver) {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", driver, false).await;
    let job = desk
        .client
        .call(
            "schedule.create",
            json!({
                "personaId": ada,
                "kind": "schedule",
                "when": chrono::Utc::now().timestamp_millis() + 86_400_000,
                "prompt": "Check the release",
            }),
        )
        .await;
    assert_eq!(job["ok"], true, "{job}");
    let job_id = job["result"]["id"].as_str().unwrap().to_owned();
    let before = chrono::Utc::now().timestamp_millis();
    let script = [
        format!(
            r#"CALL outcome {{"type":"open","message":"Watching the release","id":"rel","review_by":"7d","job_id":"{job_id}"}}"#
        ),
        r#"CALL outcome {"type":"open","message":"Watching nothing","id":"bogus","job_id":"no-such-job"}"#
            .to_string(),
    ]
    .join("\n");
    let sent = desk
        .client
        .call("session.prompt", json!({"personaId": ada, "text": script}))
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let results = desk.results(tape).await;
    says(&results, 0, "id 'rel'");
    says(&results, 1, "no live job");

    let loops = kept::of(&desk.log, &ada);
    assert_eq!(loops.len(), 1, "the refused one opened no loop: {loops:#?}");
    let watching = &loops[0];
    assert_eq!(watching.key, "rel");
    assert_eq!(watching.summary, "Watching the release");
    assert_eq!(watching.state, LoopState::Active);
    assert_eq!(watching.source, LoopSource::Outcome);
    assert_eq!(watching.job_id.as_deref(), Some(job_id.as_str()));
    let week = 7 * 86_400_000;
    let review_by = watching.review_by.expect("review_by is kept");
    assert!(
        review_by >= before + week && review_by <= chrono::Utc::now().timestamp_millis() + week,
        "{review_by}"
    );
    assert!(
        declared_on_disk(&desk, &ada)
            .iter()
            .all(|line| line["key"] != "bogus"),
        "a refused outcome leaves no line"
    );

    let cancelled = desk
        .client
        .call("schedule.cancel", json!({"id": job_id}))
        .await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let closed = kept::of(&desk.log, &ada).remove(0);
    assert_eq!(closed.state, LoopState::Done, "{closed:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_mirrors_an_open_outcome_into_its_open_loops() {
    an_open_outcome_is_an_open_loop_tracking_its_job(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_child_mirrors_an_open_outcome_into_its_open_loops() {
    an_open_outcome_is_an_open_loop_tracking_its_job(Driver::Acp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_says_what_it_is_on_and_what_it_left() {
    a_turn_says_what_it_is_on_and_what_it_left(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_child_says_what_it_is_on_and_what_it_left() {
    a_turn_says_what_it_is_on_and_what_it_left(Driver::Acp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_closes_an_open_outcome_by_its_id() {
    an_open_outcome_closes_by_its_id(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_child_closes_an_open_outcome_by_its_id() {
    an_open_outcome_closes_by_its_id(Driver::Acp).await;
}
