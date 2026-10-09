//! An ACP agent that never finishes starting, through the real core and the
//! real wire (BRO-267).
//!
//! Pork Chop's launcher hung before it reached its agent. Its start had no
//! bound, and every command the window sent after it waited behind it on the
//! one socket: Jeeves, who runs in process and has nothing to do with ACP,
//! stopped answering, and a message sent to him was drawn but never written.
//! Here both teammates share one socket, as they do in the window.
#![cfg(unix)]
mod common;

use common::stalled::{self, Client};
use hotline_core::desk::Desk;
use hotline_core::driver::acp::StartBounds;
use hotline_core::wire::Door;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TOKEN: &str = "disposable-acp-startup-test";
/// Long enough for Jeeves's whole turn to run inside it, short enough for a
/// test.
const BOUND: Duration = Duration::from_secs(6);
const QUICK: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_agent_start_fails_visibly_and_holds_up_nobody_else() {
    let root = std::env::temp_dir().join(format!("hotline-acp-startup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let pids = stalled::install(&root);
    let (base, server) = stalled::model_endpoint("At your service.").await;
    let desk = Desk::open_with_acp_start_bounds(
        &root,
        common::store(),
        StartBounds {
            known: BOUND,
            first: BOUND,
        },
    )
    .unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;

    let saved = client
        .call(
            "credential.custom_save",
            json!({"draft": {"name":"butler", "baseUrl":base, "api":"chat_completions", "models":["vendor/butler"]}}),
            QUICK,
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let cwd = root.join("workspace").to_string_lossy().into_owned();
    let made = client
        .call(
            "persona.create",
            json!({"draft": {"name":"Jeeves", "goal":"Answer", "cwd":cwd}}),
            QUICK,
        )
        .await;
    let jeeves = made["result"]["id"].as_str().unwrap().to_owned();
    let made = client
        .call(
            "persona.create",
            json!({"draft": {"name":"Pork Chop", "goal":"Code", "cwd":cwd, "backendId":stalled::BACKEND}}),
            QUICK,
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    let pork_chop = made["result"]["id"].as_str().unwrap().to_owned();
    let jeeves_tape = client.subscribe(json!({"tape":jeeves})).await;
    let pork_chop_tape = client.subscribe(json!({"tape":pork_chop})).await;

    // Pork Chop's start goes first and hangs: its command runs and never
    // answers `initialize`.
    let asked = Instant::now();
    let starting = client
        .send("session.start", json!({"personaId":pork_chop}))
        .await;
    let (group, grandchild) = stalled::spawned(&pids).await;

    // Jeeves, on the same socket, starts, is spoken to and answers while it
    // hangs, and what was said to him is on his tape.
    let started = client
        .call("session.start", json!({"personaId":jeeves}), QUICK)
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let sent = client
        .call(
            "session.prompt",
            json!({"personaId":jeeves, "text":"Is anyone there?"}),
            QUICK,
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    client
        .next_where(QUICK, |frame| {
            frame["sub"] == jeeves_tape
                && frame["event"]["kind"] == "user"
                && frame["event"]["text"] == "Is anyone there?"
        })
        .await;
    let answered = client
        .next_where(QUICK, |frame| {
            frame["sub"] == jeeves_tape && frame["event"]["kind"] == "turn"
        })
        .await;
    assert_eq!(answered["event"]["stopReason"], "end_turn", "{answered}");
    assert!(
        !client.replied(starting).await && stalled::group_alive(group),
        "Jeeves answered while Pork Chop was still starting"
    );

    // Pork Chop's start ends at the bound: failed, not hung, with the reason
    // on its tape as an error card that names the command.
    let started = client.reply(starting, BOUND + QUICK).await;
    assert!(asked.elapsed() >= BOUND, "it waited out the bound");
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["state"], "error", "{started}");
    let card = client
        .next_where(QUICK, |frame| {
            frame["sub"] == pork_chop_tape
                && frame["event"]["kind"] == "notice"
                && frame["event"]["level"] == "error"
        })
        .await;
    let text = card["event"]["text"].as_str().unwrap();
    assert!(text.contains("\"hotlineFailure\""), "{text}");
    assert!(text.contains("\"kind\":\"startup\""), "{text}");
    assert!(
        text.contains(&format!("{} didn't start within 6 s", stalled::NAME)),
        "{text}"
    );
    assert!(text.contains("/stall"), "the command is named: {text}");
    assert!(
        text.contains("resolving the toolchain"),
        "with its stderr: {text}"
    );

    // The whole process group is gone, the grandchild with it.
    assert!(stalled::group_gone(group).await, "the launcher was killed");
    // Safety: signal 0 only asks whether the process exists.
    assert_ne!(
        unsafe { libc::kill(grandchild, 0) },
        0,
        "and what it started"
    );

    // A message to Pork Chop now is written before anything waits on the
    // agent, and the start it retries fails as the turn.
    std::fs::remove_file(&pids).unwrap();
    let sent = client
        .call(
            "session.prompt",
            json!({"personaId":pork_chop, "text":"Are you up yet?"}),
            QUICK,
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    client
        .next_where(QUICK, |frame| {
            frame["sub"] == pork_chop_tape
                && frame["event"]["kind"] == "user"
                && frame["event"]["text"] == "Are you up yet?"
        })
        .await;
    let (retried, _) = stalled::spawned(&pids).await;
    let failed = client
        .next_where(BOUND + QUICK, |frame| {
            frame["sub"] == pork_chop_tape && frame["event"]["kind"] == "turn"
        })
        .await;
    assert_eq!(failed["event"]["stopReason"], "failed", "{failed}");
    assert!(
        stalled::group_gone(retried).await,
        "the retry's launcher was killed"
    );

    drop(client);
    door_task.abort();
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}
