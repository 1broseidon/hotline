use std::collections::HashSet;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Implementation};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::json;

type Client = RunningService<rmcp::RoleClient, ClientInfo>;

#[tokio::test(flavor = "multi_thread")]
async fn image_honors_the_computer_contract() {
    let Some(base) = std::env::var("TOAD_COMPUTER_URL").ok() else {
        eprintln!("skipped: set TOAD_COMPUTER_URL to run the container contract test");
        return;
    };
    let token = std::env::var("TOAD_COMPUTER_TOKEN").unwrap_or_default();
    let health = reqwest::get(format!("{base}/health"))
        .await
        .expect("health request");
    assert!(health.status().is_success(), "/health is open");

    if !token.is_empty() {
        let naked = reqwest::Client::new()
            .post(format!("{base}/mcp"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .expect("unauthenticated request");
        assert_eq!(naked.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(
            naked.json::<serde_json::Value>().await.unwrap()["error"],
            "unauthorized"
        );
    }

    let client = connect(&base, &token, "contract").await;
    let listed = client.list_all_tools().await.expect("tools/list");
    let names: HashSet<_> = listed.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(
        names,
        HashSet::from([
            "capture", "input", "browser", "shell", "files", "windows", "wait", "state"
        ])
    );

    let shell = call(
        &client,
        "shell",
        json!({"command":"sh","args":["-c","echo computer-says-42"]}),
    )
    .await;
    assert!(
        text(&shell).contains("computer-says-42"),
        "{}",
        text(&shell)
    );

    let navigated = call(
        &client,
        "browser",
        json!({"action":"navigate","url":"data:text/html,<h1>Rust computer proof</h1>"}),
    )
    .await;
    assert!(!navigated.is_error.unwrap_or(false), "{}", text(&navigated));
    let page = call(&client, "browser", json!({"action":"text"})).await;
    assert!(
        text(&page).contains("Rust computer proof"),
        "{}",
        text(&page)
    );

    let capture = call(&client, "capture", json!({})).await;
    assert!(
        capture
            .content
            .iter()
            .any(|block| block.as_image().is_some())
    );
    assert!(
        text(&capture).contains('['),
        "capture contains a window tree"
    );
    let windows = call(&client, "windows", json!({"action":"list"})).await;
    let window_list: Vec<serde_json::Value> =
        serde_json::from_str(&text(&windows)).expect("window JSON");
    assert!(
        !window_list.is_empty(),
        "Chromium is a visible desktop window"
    );

    let path = "/home/agent/contract/probe.txt";
    let put = call(
        &client,
        "files",
        json!({"action":"put","path":path,"content":"over-mcp"}),
    )
    .await;
    assert!(!put.is_error.unwrap_or(false), "{}", text(&put));
    let get = call(&client, "files", json!({"action":"get","path":path})).await;
    assert_eq!(text(&get), "over-mcp");
    let escaped = call(
        &client,
        "files",
        json!({"action":"get","path":"/etc/passwd"}),
    )
    .await;
    assert!(escaped.is_error.unwrap_or(false));
    assert!(text(&escaped).contains("path must be under"));

    let alice = connect(&base, &token, "alice").await;
    let mallory = connect(&base, &token, "mallory").await;
    let taken = call(&alice, "state", json!({"action":"control","duration":60})).await;
    assert!(text(&taken).contains("alice"));
    let blocked = call(&mallory, "input", json!({"action":"key","combo":"Escape"})).await;
    assert!(blocked.is_error.unwrap_or(false));
    assert!(text(&blocked).contains("alice"));
    let released = call(&alice, "state", json!({"action":"release"})).await;
    assert!(text(&released).contains("\"released\":true"));

    client.cancel().await.ok();
    alice.cancel().await.ok();
    mallory.cancel().await.ok();
}

async fn connect(base: &str, token: &str, holder: &str) -> Client {
    let mut headers = HeaderMap::new();
    if !token.is_empty() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    headers.insert(
        HeaderName::from_static("x-computer-holder"),
        HeaderValue::from_str(holder).unwrap(),
    );
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp")),
    );
    ClientInfo::new(
        Default::default(),
        Implementation::new("toad-computer-contract", "1"),
    )
    .serve(transport)
    .await
    .expect("MCP handshake")
}

async fn call(
    client: &Client,
    name: &str,
    arguments: serde_json::Value,
) -> rmcp::model::CallToolResult {
    client
        .peer()
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(arguments.as_object().expect("object").clone()),
        )
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|content| content.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}
