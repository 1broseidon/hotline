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
/// tool, nor `send_file` or `generate_image`, and a call to any of them is
/// refused in a sentence that says where its words go instead.
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
    let posting = [
        ("post_to_chat", json!({"text": "Found it."})),
        (
            "send_file",
            json!({"source": "workspace", "path": "notes.txt"}),
        ),
        ("generate_image", json!({"prompt": "A crane at dusk."})),
    ];
    for (tool, arguments) in posting {
        assert!(!offered.iter().any(|name| name == tool), "{offered:?}");
        let refused = tools.call(tool, &arguments).await.unwrap_err();
        assert_eq!(refused, crate::session::SUBAGENT, "{tool}");
    }
    assert_eq!(posts(&room), [] as [Value; 0]);
}

/// A job is a subagent on a schedule that the person asked for: unlike a
/// plain run it is offered the doors to the chat and posts through them,
/// once a turn, stamped with its job and from its thread. It has the home
/// thread's tools but the few that are the DM's or would schedule more.
#[tokio::test]
async fn a_jobs_thread_posts_where_a_run_may_not() {
    let room = room("post-job", Fake::new(Scripted::new(Vec::new())));
    let run = crate::contract::ScheduledRun {
        job_id: "j1".to_string(),
        kind: crate::contract::ScheduleKind::Loop,
        name: "Crane check".to_string(),
        operator_created: true,
        quiet: Some(true),
    };
    let _turn = room.open_posting(&ThreadId::job("j1"), Some(run));
    let tools = TeammateTools::new(&room, "ada").for_job("j1");
    let offered: Vec<String> = tools
        .as_dynamic()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    for tool in [
        "post_to_chat",
        "send_file",
        "generate_image",
        "cancel_schedule",
        "request_human",
        "withdraw_request",
        "message_teammate",
        "computer_status",
    ] {
        assert!(offered.iter().any(|name| name == tool), "{offered:?}");
    }
    for tool in [
        "schedule",
        "loop",
        "new_chapter",
        "resume_chapter",
        "react",
        "open_link",
    ] {
        assert!(!offered.iter().any(|name| name == tool), "{offered:?}");
        let refused = tools.call(tool, &json!({})).await.unwrap_err();
        assert!(refused.starts_with("A job has no tool"), "{refused}");
    }

    assert_eq!(
        tools
            .call("post_to_chat", &json!({"text": "The crane is down."}))
            .await
            .as_deref(),
        Ok("Posted to the chat.")
    );
    let again = tools
        .call("post_to_chat", &json!({"text": "And again."}))
        .await
        .unwrap_err();
    assert!(again.contains("one per turn"), "{again}");
    let posted = posts(&room);
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(posted[0]["scheduled"]["jobId"], "j1");
    assert_eq!(posted[0]["from"], json!({"thread": "j1", "kind": "job"}));

    let plain = TeammateTools::new(&room, "ada").for_run();
    let _run_turn = room.open_posting(&ThreadId::run("r1"), None);
    let refused = plain
        .call("post_to_chat", &json!({"text": "From a run."}))
        .await
        .unwrap_err();
    assert_eq!(refused, crate::session::SUBAGENT);
}

/// The DM's own turn is the chat: it may not post, and its file is its reply.
#[tokio::test]
async fn the_dms_own_turn_may_not_post() {
    let room = room("post-dm", Fake::new(Scripted::new(Vec::new())));
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    let refused = TeammateTools::new(&room, "ada")
        .call("post_to_chat", &json!({"text": "Hello."}))
        .await
        .unwrap_err();
    assert!(refused.contains("already the chat"), "{refused}");
    assert_eq!(posts(&room), [] as [Value; 0]);
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
    assert!(
        tools
            .call("read_thread", &json!({"thread":"dm:ada"}))
            .await
            .is_ok()
    );
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
    let read_args = json!({"thread":"dm:ada"});
    let read_in_process = tools.call("read_thread", &read_args).await.unwrap_err();
    let read_over_endpoint = connected
        .peer()
        .call_tool(
            CallToolRequestParams::new("read_thread")
                .with_arguments(read_args.as_object().unwrap().clone()),
        )
        .await
        .expect("the reader reached the handler");
    assert_eq!(read_over_endpoint.is_error, Some(true));
    assert_eq!(
        read_over_endpoint.content[0].as_text().unwrap().text,
        read_in_process
    );
    assert_eq!(read_in_process, in_process);
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
    assert_eq!(posts(&room), [] as [Value; 0]);
}
