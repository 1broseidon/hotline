//! A scripted ACP agent the harnesses spawn as a real child, so a test can
//! drive the ACP path end to end: the handshake, the desk's MCP server named
//! in `session/new`, and tool calls over it.
//!
//! It does what its prompt says. Each line `CALL <tool> <json arguments>`
//! anywhere in a prompt is one call to that tool on Hotline's MCP server, made
//! in order; the turn's one message is a `RESULTS` line followed by one line
//! per call, `<tool> ok: <text>` or `<tool> refused: <text>`. A prompt with no
//! calls is answered "On it." A REPLY_JSON line supplies a JSON string as
//! the answer, so reply-format scenarios need not echo session instructions.

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    McpServer, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Responder};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// Where Hotline's own MCP server is for a session: its URL and bearer.
#[derive(Clone)]
struct Desk {
    url: String,
    token: String,
}

type Sessions = Arc<Mutex<HashMap<String, Desk>>>;

#[tokio::main]
async fn main() {
    let sessions: Sessions = Arc::default();
    let opened = sessions.clone();
    let histories: Arc<Mutex<HashMap<String, Vec<String>>>> = Arc::default();
    let transport = ByteStreams::new(
        tokio::io::stdout().compat_write(),
        tokio::io::stdin().compat(),
    );
    let running = Agent
        .builder()
        .name("hotline-acp-fixture")
        .on_receive_request(
            async |request: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: NewSessionRequest, responder: Responder<NewSessionResponse>, _cx| {
                let opened = opened.clone();
                async move {
                    let id = uuid::Uuid::new_v4().to_string();
                    if let Some(desk) = hotline_server(&request.mcp_servers) {
                        opened.lock().unwrap().insert(id.clone(), desk);
                    }
                    responder.respond(NewSessionResponse::new(SessionId::new(id)))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: PromptRequest,
                  responder: Responder<PromptResponse>,
                  cx: ConnectionTo<Client>| {
                let histories = histories.clone();
                let desk = sessions
                    .lock()
                    .unwrap()
                    .get(&request.session_id.0.to_string())
                    .cloned();
                async move {
                    let said: String = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let session = request.session_id.clone();
                    let replying = cx.clone();
                    // The calls run off the connection's own loop, which has
                    // to stay free to carry the session's other messages.
                    cx.spawn(async move {
                        let reply = answer(&said, desk).await;
                        if let Some(path) = said
                            .lines()
                            .find_map(|line| line.trim().strip_prefix("WAIT_FOR "))
                        {
                            while !std::path::Path::new(path).exists() {
                                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            }
                        }
                        let reply = {
                            let mut histories = histories.lock().unwrap();
                            let history = histories.entry(session.0.to_string()).or_default();
                            history.push(said.clone());
                            if let Some(reply) = said
                                .lines()
                                .find_map(|line| line.trim().strip_prefix("REPLY_JSON "))
                            {
                                serde_json::from_str::<String>(reply)
                                    .expect("REPLY_JSON is a JSON string")
                            } else if said.contains("ECHO_HISTORY") {
                                history.join("\n")
                            } else {
                                reply
                            }
                        };
                        replying.send_notification(SessionNotification::new(
                            session,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(reply)),
                            )),
                        ))?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    })?;
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(transport)
        .await;
    if let Err(error) = running {
        eprintln!("hotline-acp-fixture ended: {error}");
    }
}

/// The server Hotline named for itself in `session/new`.
fn hotline_server(servers: &[McpServer]) -> Option<Desk> {
    servers.iter().find_map(|server| match server {
        McpServer::Http(http) if http.name == "hotline" => {
            let token = http.headers.iter().find_map(|header| {
                (header.name == "Authorization")
                    .then(|| header.value.trim_start_matches("Bearer ").to_string())
            })?;
            Some(Desk {
                url: http.url.clone(),
                token,
            })
        }
        _ => None,
    })
}

/// The calls a prompt asks for, in order.
fn calls(said: &str) -> Vec<(String, Value)> {
    said.lines()
        .filter_map(|line| {
            let rest = &line[line.find("CALL ")? + "CALL ".len()..];
            let (tool, arguments) = rest.split_once(' ')?;
            Some((tool.to_string(), serde_json::from_str(arguments).ok()?))
        })
        .collect()
}

/// Makes the calls the prompt asks for and says what came back.
async fn answer(said: &str, desk: Option<Desk>) -> String {
    let calls = calls(said);
    if calls.is_empty() {
        return "On it.".to_string();
    }
    let Some(desk) = desk else {
        return "RESULTS\nno hotline server was named".to_string();
    };
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(desk.url).auth_header(desk.token),
    );
    let client = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("hotline-acp-fixture", env!("CARGO_PKG_VERSION")),
    );
    let connected = match client.serve(transport).await {
        Ok(connected) => connected,
        Err(error) => return format!("RESULTS\nthe hotline server refused: {error}"),
    };
    let mut lines = vec!["RESULTS".to_string()];
    for (tool, arguments) in calls {
        let mut request = CallToolRequestParams::new(tool.clone());
        if let Value::Object(arguments) = arguments {
            request = request.with_arguments(arguments);
        }
        lines.push(match connected.peer().call_tool(request).await {
            Ok(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|block| block.as_text().map(|text| text.text.clone()))
                    .collect::<Vec<_>>()
                    .join(" ");
                let outcome = if result.is_error == Some(true) {
                    "refused"
                } else {
                    "ok"
                };
                format!("{tool} {outcome}: {text}")
            }
            Err(error) => format!("{tool} failed: {error}"),
        });
    }
    connected.cancel().await.ok();
    lines.join("\n")
}
