//! Living open loops over the real wire: `loops.list`, `{"loops": personaId}`
//! live snapshots, `loops.dismiss`, and scheduled job cancellation closing a
//! bound loop.

use crate::common::{ANSWER, Client, open_desk};
use hotline_core::{
    contract::{LoopSource, LoopState, OpenLoop},
    log::{Log, StreamId},
    wire::Door,
};
use hotline_store::loops as kept;
use serde_json::{Map, Value, json};
use std::sync::Arc;

fn sample_loop(
    id: &str,
    key: &str,
    summary: &str,
    state: LoopState,
    job_id: Option<&str>,
) -> OpenLoop {
    OpenLoop {
        id: id.into(),
        key: key.into(),
        summary: summary.into(),
        state,
        source: LoopSource::Outcome,
        created_at: 1_000,
        updated_at: 2_000,
        review_by: None,
        job_id: job_id.map(str::to_string),
        last_chapter_id: None,
        closed_at: None,
        newer: Map::new(),
    }
}

#[tokio::test]
async fn open_loops_list_subscribe_dismiss_and_job_cancel_auto_close_over_wire() {
    let root = tempfile::tempdir().unwrap();
    let seed = Log::open(root.path());
    seed.append(
        &StreamId::Room,
        &json!({
            "kind": "persona",
            "id": "ada",
            "name": "Ada",
            "goal": "Help out",
            "backendId": "hotline",
            "cwd": root.path(),
            "mcpPolicy": {"mode": "none", "serverIds": []},
            "backgroundWork": true,
            "sessionCheckpoints": [],
            "createdAt": 1,
            "updatedAt": 1
        }),
    )
    .unwrap();
    drop(seed);

    let desk = open_desk(root.path()).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), "loops-test".into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());

    let mut client = Client::connect(port, "loops-test").await;

    // Create a scheduled job so we can bind a loop to its job_id and cancel it.
    let created = client
        .call(
            "schedule.create",
            json!({
                "personaId": "ada",
                "kind": "schedule",
                "prompt": "Check release status",
                "when": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 + 3_600_000
            }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let job_id = created["result"]["id"].as_str().unwrap().to_string();

    // Seed two open loops: one stale, one active bound to the scheduled job.
    let stale = sample_loop(
        "l-stale",
        "ca-filing",
        "File CA report",
        LoopState::Stale,
        None,
    );
    let tracked = sample_loop(
        "l-job",
        "release-watch",
        "Watching release job",
        LoopState::Active,
        Some(&job_id),
    );
    log.append(&StreamId::Loops("ada".into()), &kept::line(&tracked))
        .unwrap();
    log.append(&StreamId::Loops("ada".into()), &kept::line(&stale))
        .unwrap();

    // loops.list returns active first, then stale.
    let listed = client.call("loops.list", json!({"personaId": "ada"})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let items = listed["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], "l-job");
    assert_eq!(items[1]["id"], "l-stale");

    // Subscribe to {"loops": "ada"} and verify initial snapshot.
    let sub = client.subscribe(json!({"loops": "ada"})).await;
    let first_snap = client
        .next_where(ANSWER, |frame| {
            frame["sub"] == sub && frame["snapshot"].is_array()
        })
        .await;
    assert_eq!(first_snap["snapshot"].as_array().unwrap().len(), 2);

    // Dismissing l-stale archives it and pushes an updated snapshot on the subscription.
    let dismissed = client
        .call(
            "loops.dismiss",
            json!({"personaId": "ada", "id": "l-stale"}),
        )
        .await;
    assert_eq!(dismissed["ok"], true, "{dismissed}");
    assert_eq!(dismissed["result"]["state"], "archived");

    let after_dismiss = client
        .next_where(ANSWER, |frame| {
            frame["sub"] == sub
                && frame["snapshot"].as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["id"] == "l-stale" && row["state"] == "archived")
                })
        })
        .await;
    assert!(after_dismiss["snapshot"].is_array());

    // Cancelling the bound scheduled job automatically marks l-job as done!
    let cancelled = client.call("schedule.cancel", json!({"id": job_id})).await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");

    let after_cancel = client
        .next_where(ANSWER, |frame| {
            frame["sub"] == sub
                && frame["snapshot"].as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["id"] == "l-job" && row["state"] == "done")
                })
        })
        .await;
    let final_rows: Vec<&Value> = after_cancel["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert!(
        final_rows
            .iter()
            .any(|r| r["id"] == "l-job" && r["state"] == "done")
    );
    assert!(
        final_rows
            .iter()
            .any(|r| r["id"] == "l-stale" && r["state"] == "archived")
    );

    server.abort();
}
