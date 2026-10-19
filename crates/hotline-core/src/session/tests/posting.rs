//! `post_to_chat`'s gate through the handler both drivers reach: what no
//! headless harness can put an agent in front of (a subagent run, a lease
//! revoked while the turn is open) and a call that lands between turns.

use super::*;
use crate::driver::CapabilityEpoch;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

fn posts(room: &Room) -> Vec<Value> {
    tape(room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "agent")
        .collect()
}

/// A run reports to the teammate that launched it: it is not offered the
/// tool, and a call to it is refused in a sentence that says where its words
/// go instead.
#[tokio::test]
async fn a_subagent_run_may_not_post() {
    let room = room("post-run", Fake::new(Scripted::new(Vec::new())));
    let run = ThreadId::run("r1");
    let _turn = room.open_posting(&run, None);
    let tools = TeammateTools::new(&room, "ada").for_run();

    let offered: Vec<String> = tools
        .as_dynamic()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert!(
        !offered.iter().any(|name| name == "post_to_chat"),
        "{offered:?}"
    );
    let refused = tools
        .call("post_to_chat", &json!({"text": "Found it."}))
        .await
        .unwrap_err();
    assert_eq!(refused, crate::session::SUBAGENT);
    assert!(posts(&room).is_empty());
}

/// The thread's lease is checked when the call lands: a work thread whose
/// authority was revoked mid-turn posts nothing, whether the call comes in
/// process or over the endpoint an ACP child is handed.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_lease_refuses_a_post_in_process_and_over_the_endpoint() {
    let room = room("post-revoked", Fake::new(Scripted::new(Vec::new())));
    let epoch = CapabilityEpoch::default();
    let tools = TeammateTools::new(&room, "ada")
        .for_work("s1")
        .with_capability(epoch.lease());
    let first = room.open_posting(&ThreadId::side("s1"), None);
    assert_eq!(
        tools
            .call("post_to_chat", &json!({"text": "The build is green."}))
            .await
            .as_deref(),
        Ok("Posted to the chat.")
    );
    drop(first);

    let _second = room.open_posting(&ThreadId::side("s1"), None);
    let served = crate::mcp::server::serve(tools.clone()).await.unwrap();
    epoch.invalidate();
    let in_process = tools
        .call("post_to_chat", &json!({"text": "Revoked in process."}))
        .await
        .unwrap_err();
    let connected = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("test", env!("CARGO_PKG_VERSION")),
    )
    .serve(StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(served.url()).auth_header(served.token()),
    ))
    .await
    .expect("the handshake succeeded");
    let over_the_endpoint = connected
        .peer()
        .call_tool(
            CallToolRequestParams::new("post_to_chat").with_arguments(
                json!({"text": "Revoked over the endpoint."})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("the call reached the handler");
    connected.cancel().await.ok();

    assert_eq!(over_the_endpoint.is_error, Some(true));
    let said = over_the_endpoint.content[0].as_text().unwrap().text.clone();
    assert_eq!(said, in_process);
    let texts: Vec<Value> = posts(&room).iter().map(|e| e["text"].clone()).collect();
    assert_eq!(texts, [json!("The build is green.")]);
}

/// A call that lands with no turn of its thread in flight (an agent's own
/// work after its turn ended) is refused rather than counted against a turn.
#[tokio::test]
async fn a_post_between_turns_is_refused() {
    let room = room("post-between", Fake::new(Scripted::new(Vec::new())));
    let refused = TeammateTools::new(&room, "ada")
        .for_work("s1")
        .call("post_to_chat", &json!({"text": "Late."}))
        .await
        .unwrap_err();
    assert!(refused.contains("turn in progress"), "{refused}");
    assert!(posts(&room).is_empty());
}
