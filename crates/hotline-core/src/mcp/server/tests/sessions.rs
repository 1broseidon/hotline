//! The MCP sessions a child opens on its endpoint, over real HTTP: how long
//! one lives when the child goes quiet, and that it ends with the endpoint.

use super::*;
use crate::test_support::until;
use std::time::Duration;

/// A revision with the `initialize` handshake, which is what gives a client a
/// session; 2026-07-28 has neither.
const SESSION_PROTOCOL: &str = "2025-06-18";

/// One JSON-RPC message posted to the endpoint as a child's MCP client posts
/// it, inside `session` once there is one.
async fn post(served: &Served, session: Option<&str>, message: Value) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(served.url())
        .bearer_auth(served.token())
        .header("accept", "application/json, text/event-stream")
        .json(&message);
    if let Some(session) = session {
        request = request
            .header("mcp-session-id", session)
            .header("mcp-protocol-version", SESSION_PROTOCOL);
    }
    request.send().await.expect("the endpoint answered")
}

/// Opens a session the way a child does and sends nothing after it: what a
/// child sitting on a card looks like, and what a killed child leaves.
async fn open_session(served: &Served) -> String {
    let initialized = post(
        served,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": SESSION_PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" },
            },
        }),
    )
    .await;
    let session = initialized
        .headers()
        .get("mcp-session-id")
        .expect("an initialize opens a session")
        .to_str()
        .unwrap()
        .to_string();
    initialized.text().await.unwrap();
    let ready = post(
        served,
        Some(&session),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    assert_eq!(ready.status(), reqwest::StatusCode::ACCEPTED);
    session
}

/// A child can wait on a permission card for longer than rmcp's default five
/// idle minutes, and one that does not open a new session after that would
/// have no Hotline tools for the rest of its life.
#[tokio::test]
async fn a_session_idle_past_five_minutes_still_answers() {
    let room = room_with_a_conversation("idle");
    let served = serve(tools(&room)).await.unwrap();
    let session = open_session(&served).await;

    // A paused clock moves only when every task is waiting, and then to the
    // next timer, so an idle timer fires and its session is closed before
    // this sleep returns.
    tokio::time::pause();
    tokio::time::sleep(Duration::from_secs(301)).await;
    tokio::time::resume();

    let listed = post(
        &served,
        Some(&session),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    assert_eq!(listed.status(), reqwest::StatusCode::OK);
    let body = listed.text().await.unwrap();
    assert!(body.contains(SEARCH_THREAD), "{body}");
}

/// The driver drops its endpoint when its child ends, and the child's
/// sessions must go with it: no idle timer ends a session, so a killed
/// child's would otherwise run, holding its teammate's tools, for as long as
/// the desk does. Each copy of the tools holds a weak handle on the room,
/// which is how this sees them.
#[tokio::test]
async fn dropping_the_endpoint_ends_its_sessions() {
    let room = room_with_a_conversation("dropped");
    // The room's start-up tasks take and drop handles of their own first.
    tokio::task::yield_now().await;
    let before = Arc::weak_count(&room);
    let served = serve(tools(&room)).await.unwrap();
    let endpoint_alone = Arc::weak_count(&room);
    open_session(&served).await;
    assert!(
        Arc::weak_count(&room) > endpoint_alone,
        "the session holds no copy of the tools, so this test sees nothing"
    );

    drop(served);
    until("the session ends with its endpoint", || {
        Arc::weak_count(&room) <= before
    })
    .await;
}
