//! A scripted ACP agent the harnesses spawn as a real child, so a test can
//! drive the ACP path end to end: the handshake, the desk's MCP server named
//! in `session/new`, and tool calls over it.
//!
//! It does what its prompt says. Each line `CALL <tool> <json arguments>`
//! anywhere in a prompt is one call to that tool on Hotline's MCP server, made
//! in order; the turn's one message is a `RESULTS` line followed by one line
//! per call, `<tool> ok: <text>` or `<tool> refused: <text>`. A prompt with no
//! calls is answered "On it."
//!
//! With `--cli` anywhere in its arguments it is instead the agent's own command
//! line for an account, `auth status [--json]`, answered from the folder in
//! `CLAUDE_CONFIG_DIR`. Every run there writes the names of its environment,
//! never a value, to `<that folder>/fixture-status-env.json`; without the
//! variable it writes nothing, so it never reaches the home folder.

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
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
    let args: Vec<OsString> = std::env::args_os().collect();
    if let Some(at) = args.iter().position(|arg| arg == "--cli") {
        std::process::exit(cli(&args[at + 1..]));
    }
    let sessions: Sessions = Arc::default();
    let opened = sessions.clone();
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

fn cli(args: &[OsString]) -> i32 {
    let args: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    match words.as_slice() {
        ["auth", "status"] | ["auth", "status", "--json"] => auth_status(dir.as_deref()),
        _ => 2,
    }
}

fn report_env_names(dir: &Path, run: &str) {
    let mut names: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    let report = serde_json::to_vec(&names).expect("names serialize");
    std::fs::write(dir.join(format!("fixture-{run}-env.json")), report)
        .expect("the account folder is writable");
}

fn auth_status(dir: Option<&Path>) -> i32 {
    let Some(dir) = dir else {
        println!(r#"{{"loggedIn":false}}"#);
        return 1;
    };
    report_env_names(dir, "status");
    if std::env::var("FIXTURE_STATUS_MODE").as_deref() == Ok("hang") {
        std::thread::sleep(Duration::from_secs(600));
    }
    let config_directory = dir.to_string_lossy();
    let answer = match std::env::var("FIXTURE_AUTH").as_deref() {
        Ok("api_key") => json!({
            "loggedIn": true,
            "authMethod": "api_key",
            "apiProvider": "firstParty",
            "apiKeySource": "ANTHROPIC_API_KEY",
            "configDirectory": config_directory,
        }),
        Ok("bedrock") => json!({
            "loggedIn": true,
            "authMethod": "third_party",
            "apiProvider": "bedrock",
            "configDirectory": config_directory,
        }),
        _ => {
            let credentials = dir.join(".credentials.json");
            let logged_in = credentials.exists();
            let email = std::fs::read(&credentials)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|saved| saved["email"].as_str().map(str::to_string));
            json!({
                "loggedIn": logged_in,
                "authMethod": if logged_in { "claude.ai" } else { "none" },
                "apiProvider": "firstParty",
                "email": email,
                "subscriptionType": "max",
                "configDirectory": config_directory,
            })
        }
    };
    println!("{answer}");
    if answer["loggedIn"] == true { 0 } else { 1 }
}
