//! Toad as an MCP **client**: the servers the room knows, the grant a
//! teammate gets, and the live connections a session holds.
//!
//! Servers are defined once, under the room setting `mcpServers`, and
//! teammates reference them by id. That split is deliberate: a server is a
//! piece of infrastructure with a command in it, while which teammate may
//! use it is a question about that teammate. An id that no longer names a
//! server is dropped rather than treated as an error — deleting a server
//! should not break every teammate that referenced it — and the drop is
//! recorded on the ledger so it is not silent.
//!
//! A half-written entry in settings costs that one server, never every
//! teammate's tools. A server whose env is not a map of strings is refused
//! — a non-object env, or a value that is not a string, named on the
//! ledger — rather than started without those variables. OAuth and
//! static-header HTTP are a later task: those
//! servers are refused with a sentence saying why, not connected with a
//! dead credential. A server that dies after it was attached is the same
//! honesty later: the next call that hits a dead transport marks that
//! origin's rows absent and says so once on the tape.
//!
//! Toad's own teammate tools are the other half of MCP, and they live
//! in [`server`].

pub mod server;
mod tool;

use tool::Watch;
pub use tool::{CallError, McpTool};

use crate::contract::{McpPolicy, PolicyMode};
use rmcp::ServiceExt;
use rmcp::model::{ClientInfo, Implementation};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;
use tokio::process::Command;

/// How long a server may take to finish the handshake and list its tools
/// before the row is absent rather than the session hanging.
const HANDSHAKE: Duration = Duration::from_secs(20);

/// One MCP server the room knows, after a settings entry has been
/// normalised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    /// Why this server must not be started. A non-string env value is the
    /// case we have: starting the process without that variable is worse
    /// than not starting it, so the ledger names the key instead.
    pub refuse: Option<String>,
}

/// How Toad reaches a server. Auth that this build cannot honour still
/// parses, so the ledger can name the server it refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    Http {
        url: String,
        auth: HttpAuth,
    },
}

/// HTTP authentication as settings store it. Only [`HttpAuth::None`] is
/// connected in this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpAuth {
    None,
    Static { header_names: Vec<String> },
    Oauth,
}

/// A server the grant named that could not be attached, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedServer {
    pub id: String,
    pub name: String,
    pub reason: String,
}

/// Live clients for one session, and the tools they listed.
///
/// The running services stay here for as long as the session: dropping them
/// closes the transport, which kills a stdio child, and dropping the groups
/// beside them reaches everything that child started. Tools clone a peer
/// handle; they are only as live as this value.
pub struct Connections {
    pub tools: Vec<McpTool>,
    pub failed: Vec<FailedServer>,
    _live: Vec<RunningService<rmcp::RoleClient, ClientInfo>>,
    /// The process groups the stdio servers were spawned into, killed when
    /// this value goes. See [`ProcessGroup`].
    _groups: Vec<ProcessGroup>,
}

/// A stdio server's process group, killed when this is dropped.
///
/// rmcp's child transport kills the process it spawned and nothing else, and
/// the command in an `mcpServers` entry is very often a launcher — `npx -y
/// some-server`, `uvx …` — whose real server is a grandchild. Killing only the
/// wrapper reparents that grandchild to pid 1, where it holds the port, the
/// file locks and the memory for as long as Toad runs. So the child is spawned
/// into a group of its own and the group is what a stop reaches, the same way
/// the ACP child and the shell tool do it.
struct ProcessGroup {
    #[cfg_attr(not(unix), allow(dead_code))]
    id: Option<u32>,
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            // Safety: `killpg` reads no memory, and the group is the one this
            // connection made with `process_group(0)`.
            unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
        }
    }
}

/// The servers this teammate's policy selects, and the ids it named that
/// no longer exist.
pub struct Grant {
    pub servers: Vec<McpServer>,
    pub missing: Vec<String>,
}

/// The server list stored under `mcpServers`, with anything unusable
/// dropped. Settings are a file a person can edit.
pub fn normalize_servers(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(normalize_server)
        .collect()
}

/// Typed servers from the room's settings fold. A missing or unusable
/// `mcpServers` key is no servers, not an error.
pub fn servers(settings: &Map<String, Value>) -> Vec<McpServer> {
    settings
        .get("mcpServers")
        .map(normalize_servers)
        .unwrap_or_default()
        .iter()
        .filter_map(parse_server)
        .collect()
}

/// Which of `available` this policy actually gets.
///
/// An id that no longer names a server is listed in [`Grant::missing`]
/// rather than treated as a start failure: deleting a server should not
/// break every teammate that referenced it, and the ledger is where the
/// drop is said out loud.
pub fn grant(available: &[McpServer], policy: &McpPolicy) -> Grant {
    match policy.mode {
        PolicyMode::None => Grant {
            servers: Vec::new(),
            missing: Vec::new(),
        },
        PolicyMode::All => Grant {
            servers: available.to_vec(),
            missing: Vec::new(),
        },
        PolicyMode::Some => {
            let known: HashMap<&str, &McpServer> = available
                .iter()
                .map(|server| (server.id.as_str(), server))
                .collect();
            let mut servers = Vec::new();
            let mut missing = Vec::new();
            for id in &policy.server_ids {
                match known.get(id.as_str()) {
                    Some(server) => servers.push((*server).clone()),
                    None => missing.push(id.clone()),
                }
            }
            Grant { servers, missing }
        }
    }
}

/// Connect every granted server. A refusal or a handshake failure becomes a
/// [`FailedServer`]; the rest list their tools. `persona_id` is who the
/// ledger names when a transport dies after this, so a silent mid-session
/// absence is still a named one.
pub async fn connect(persona_id: &str, servers: &[McpServer]) -> Connections {
    let mut tools = Vec::new();
    let mut failed = Vec::new();
    let mut live = Vec::new();
    let watch = Watch::new(persona_id);
    let mut groups = Vec::new();
    for server in servers {
        match connect_one(server).await {
            Ok((client, listed, group)) => {
                let peer = client.peer().clone();
                for definition in listed {
                    tools.push(McpTool::new(
                        &server.id,
                        &server.name,
                        definition,
                        peer.clone(),
                        watch.clone(),
                    ));
                }
                live.push(client);
                groups.extend(group);
            }
            Err(reason) => failed.push(FailedServer {
                id: server.id.clone(),
                name: server.name.clone(),
                reason,
            }),
        }
    }
    Connections {
        tools,
        failed,
        _live: live,
        _groups: groups,
    }
}

/// What the ledger says about a policy id that no longer names a server.
///
/// Both drivers write this row, and it has to read the same on either: the
/// question a person asks is "why does this teammate not have that tool",
/// and the answer does not depend on which agent they asked it about.
pub fn missing_reason(id: &str) -> String {
    format!(
        "this teammate's MCP policy names the server {id}, which no longer exists in app settings — every tool it supplied is gone"
    )
}

/// Why this build cannot hand this server to an agent, or `None` when it can.
///
/// One sentence, in one place, because two agents refuse for the same reason:
/// Toad connects the server itself for the in-process agent and names it in
/// `session/new` for a child, and neither can honour a credential this build
/// does not keep.
pub fn unsupported(server: &McpServer) -> Option<String> {
    if let Some(reason) = &server.refuse {
        return Some(reason.clone());
    }
    match &server.transport {
        McpTransport::Http {
            auth: HttpAuth::Static { .. },
            ..
        } => Some(
            "Static-header HTTP servers are a later task; this server was not connected."
                .to_string(),
        ),
        McpTransport::Http {
            auth: HttpAuth::Oauth,
            ..
        } => {
            Some("OAuth HTTP servers are a later task; this server was not connected.".to_string())
        }
        _ => None,
    }
}

/// One server, connected. A stdio server also hands back the group it was
/// spawned into, which is what the caller has to hold on to.
async fn connect_one(
    server: &McpServer,
) -> Result<
    (
        RunningService<rmcp::RoleClient, ClientInfo>,
        Vec<rmcp::model::Tool>,
        Option<ProcessGroup>,
    ),
    String,
> {
    if let Some(refusal) = unsupported(server) {
        return Err(refusal);
    }
    // The auth this build cannot honour was refused above, so what is left of
    // an HTTP server here is a URL.
    match &server.transport {
        McpTransport::Http { url, .. } => {
            let transport = StreamableHttpClientTransport::from_uri(url.clone());
            let (client, listed) = handshake(toad_client().serve(transport)).await?;
            Ok((client, listed, None))
        }
        McpTransport::Stdio { command, args, env } => {
            let transport = TokioChildProcess::new(Command::new(command).configure(|cmd| {
                cmd.args(args);
                cmd.envs(env);
                // Windows has no process group to put it in, so the child
                // itself is all rmcp's own kill can reach there.
                #[cfg(unix)]
                cmd.process_group(0);
            }))
            .map_err(|error| {
                format!(
                    "{} ({}) could not be started: {error}",
                    server.name, server.id
                )
            })?;
            let group = ProcessGroup { id: transport.id() };
            let (client, listed) = handshake(toad_client().serve(transport)).await?;
            Ok((client, listed, Some(group)))
        }
    }
}

async fn handshake<E>(
    serve: impl Future<Output = Result<RunningService<rmcp::RoleClient, ClientInfo>, E>>,
) -> Result<
    (
        RunningService<rmcp::RoleClient, ClientInfo>,
        Vec<rmcp::model::Tool>,
    ),
    String,
>
where
    E: std::fmt::Display,
{
    let client = tokio::time::timeout(HANDSHAKE, serve)
        .await
        .map_err(|_| "the server did not finish the MCP handshake in time".to_string())?
        .map_err(|error| error.to_string())?;
    let listed = tokio::time::timeout(HANDSHAKE, client.list_all_tools())
        .await
        .map_err(|_| "the server did not list its tools in time".to_string())?
        .map_err(|error| error.to_string())?;
    Ok((client, listed))
}

fn toad_client() -> ClientInfo {
    ClientInfo::new(
        Default::default(),
        Implementation::new("Toad", env!("CARGO_PKG_VERSION")),
    )
}

fn normalize_server(value: &Value) -> Option<Value> {
    let candidate = value.as_object()?;
    let id = candidate
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = candidate
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return None;
    }

    if candidate.get("type").and_then(Value::as_str) == Some("http") {
        let url = candidate
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if url.is_empty() {
            return None;
        }
        let legacy_headers = string_map(candidate.get("headers"));
        let auth = normalize_http_auth(candidate.get("auth"), legacy_headers);
        return Some(json!({ "id": id, "type": "http", "name": name, "url": url, "auth": auth }));
    }

    let command = candidate
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return None;
    }
    let args: Vec<&str> = candidate
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut server =
        json!({ "id": id, "type": "stdio", "name": name, "command": command, "args": args });
    // Keep env even when it is not an object of strings, so parse can
    // refuse the server rather than starting it without the variables.
    if let Some(env) = candidate.get("env") {
        server
            .as_object_mut()
            .expect("just built as an object")
            .insert("env".to_string(), env.clone());
    }
    Some(server)
}

fn normalize_http_auth(value: Option<&Value>, legacy: Option<&Map<String, Value>>) -> Value {
    if let Some(legacy) = legacy {
        let header_names: Vec<String> = legacy.keys().cloned().collect();
        return json!({ "mode": "static", "headerNames": header_names });
    }
    let Some(candidate) = value.and_then(Value::as_object) else {
        return json!({ "mode": "none" });
    };
    match candidate.get("mode").and_then(Value::as_str) {
        Some("static") => {
            let header_names: Vec<String> = candidate
                .get("headerNames")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect();
            json!({ "mode": "static", "headerNames": header_names })
        }
        Some("oauth") => {
            let scopes: Vec<String> = unique_keep_first(
                candidate
                    .get("scopes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|scope| !scope.is_empty())
                    .map(str::to_string),
            );
            let resource = candidate
                .get("resource")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let raw_client = candidate.get("client").and_then(Value::as_object);
            let method = raw_client
                .and_then(|client| client.get("tokenEndpointAuthMethod"))
                .and_then(Value::as_str)
                .filter(|method| {
                    matches!(
                        *method,
                        "none" | "client_secret_basic" | "client_secret_post"
                    )
                });
            let client = raw_client
                .and_then(|client| client.get("clientId").and_then(Value::as_str))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|client_id| {
                    let mut object = Map::new();
                    object.insert("clientId".to_string(), json!(client_id));
                    if let Some(method) = method {
                        object.insert("tokenEndpointAuthMethod".to_string(), json!(method));
                    }
                    Value::Object(object)
                });
            let mut auth = Map::new();
            auth.insert("mode".to_string(), json!("oauth"));
            auth.insert("scopes".to_string(), json!(scopes));
            if let Some(resource) = resource {
                auth.insert("resource".to_string(), json!(resource));
            }
            if let Some(client) = client {
                auth.insert("client".to_string(), client);
            }
            Value::Object(auth)
        }
        _ => json!({ "mode": "none" }),
    }
}

fn parse_server(value: &Value) -> Option<McpServer> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?.to_string();
    let name = object.get("name")?.as_str()?.to_string();
    let transport = match object.get("type").and_then(Value::as_str) {
        Some("http") => {
            let url = object.get("url")?.as_str()?.to_string();
            let auth = match object
                .get("auth")
                .and_then(Value::as_object)
                .and_then(|auth| auth.get("mode"))
                .and_then(Value::as_str)
            {
                Some("static") => {
                    let header_names = object
                        .get("auth")
                        .and_then(Value::as_object)
                        .and_then(|auth| auth.get("headerNames"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                    HttpAuth::Static { header_names }
                }
                Some("oauth") => HttpAuth::Oauth,
                _ => HttpAuth::None,
            };
            McpTransport::Http { url, auth }
        }
        _ => {
            let command = object.get("command")?.as_str()?.to_string();
            let args = object
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let (env, refuse) = stdio_env(object.get("env"));
            return Some(McpServer {
                id,
                name,
                transport: McpTransport::Stdio { command, args, env },
                refuse,
            });
        }
    };
    Some(McpServer {
        id,
        name,
        transport,
        refuse: None,
    })
}

/// Env values have to be strings: a number in the map used to drop the whole
/// env, which starts the server without the token that number was standing
/// in for.
fn stdio_env(value: Option<&Value>) -> (HashMap<String, String>, Option<String>) {
    let Some(value) = value else {
        return (HashMap::new(), None);
    };
    let Some(object) = value.as_object() else {
        return (
            HashMap::new(),
            Some("env is not a map of strings; the server was not started.".to_string()),
        );
    };
    let mut env = HashMap::new();
    for (key, value) in object {
        match value.as_str() {
            Some(text) => {
                env.insert(key.clone(), text.to_string());
            }
            None => {
                return (
                    HashMap::new(),
                    Some(format!(
                        "The env value for {key} is not a string; the server was not started."
                    )),
                );
            }
        }
    }
    (env, None)
}

fn string_map(value: Option<&Value>) -> Option<&Map<String, Value>> {
    let object = value?.as_object()?;
    object.values().all(Value::is_string).then_some(object)
}

fn unique_keep_first(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::ServerHandler;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    };
    use rmcp::service::RequestContext;
    use serde_json::json;
    use std::future::Future;

    fn shout_schema() -> std::sync::Arc<rmcp::model::JsonObject> {
        let schema = json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"],
        });
        std::sync::Arc::new(schema.as_object().unwrap().clone())
    }

    struct Echo;

    impl ServerHandler for Echo {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        }

        fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_ {
            std::future::ready(Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "shout",
                "Echo the text back in upper case.",
                shout_schema(),
            )])))
        }

        fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<CallToolResponse, rmcp::ErrorData>> + Send + '_ {
            let text = request
                .arguments
                .as_ref()
                .and_then(|args| args.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if text == "fail" {
                return std::future::ready(Ok(CallToolResult::error(vec![ContentBlock::text(
                    "the tool refused",
                )])
                .into()));
            }
            if text == "rpc" {
                return std::future::ready(Err(rmcp::ErrorData::invalid_params(
                    "the tool rejected the arguments",
                    None,
                )));
            }
            std::future::ready(Ok(CallToolResult::success(vec![ContentBlock::text(
                text.to_uppercase(),
            )])
            .into()))
        }
    }

    async fn echo_on_duplex(
        persona_id: &str,
    ) -> (
        tokio::task::JoinHandle<()>,
        rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>,
        McpTool,
    ) {
        let (client_to_server, server_from_client) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let _ = Echo
                .serve(server_from_client)
                .await
                .expect("echo server starts")
                .waiting()
                .await;
        });
        let client = toad_client()
            .serve(client_to_server)
            .await
            .expect("client handshake");
        let listed = client.list_all_tools().await.expect("tools listed");
        let tool = McpTool::new(
            "echo",
            "Echo",
            listed.into_iter().next().unwrap(),
            client.peer().clone(),
            Watch::new(persona_id),
        );
        (server, client, tool)
    }

    fn publish_echo(persona_id: &str) {
        use crate::contract::{AgentKind, ToolSourceKind};
        use crate::session::ledger::ToolLedger;
        let mut ledger = ToolLedger::new(persona_id, AgentKind::Pi, "pi");
        ledger
            .verified(
                ToolSourceKind::Mcp,
                "echo",
                "echo__shout",
                "attached from the echo MCP server",
            )
            .verified(
                ToolSourceKind::Mcp,
                "echo",
                "echo__whisper",
                "attached from the echo MCP server",
            )
            .verified(ToolSourceKind::Builtin, "pi", "read", "a built-in")
            .publish();
    }

    fn echo_row(persona_id: &str, name: &str) -> crate::contract::ToolLedgerRow {
        crate::session::ledger::teammate_tools(persona_id)
            .expect("published")
            .rows
            .into_iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("{name} is on the ledger"))
    }

    #[test]
    fn a_hand_edited_bad_entry_costs_that_entry_not_the_list() {
        let raw = json!([
            { "id": "no-name", "type": "stdio", "command": "echo" },
            { "id": "good", "type": "stdio", "name": "Good", "command": "run" },
            { "id": "no-url", "type": "http", "name": "Remote" },
        ]);
        let servers = normalize_servers(&raw);
        let ids: Vec<&str> = servers
            .iter()
            .map(|server| server["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["good"]);
    }

    #[test]
    fn a_legacy_header_bearing_server_normalises_with_no_header_key() {
        let raw = json!([{
            "id": "srv1",
            "type": "http",
            "name": "Legacy",
            "url": "https://example.test",
            "headers": { "Authorization": "Bearer secret" },
        }]);
        let server = &normalize_servers(&raw)[0];
        assert_eq!(
            *server,
            json!({
                "id": "srv1",
                "type": "http",
                "name": "Legacy",
                "url": "https://example.test",
                "auth": { "mode": "static", "headerNames": ["Authorization"] },
            })
        );
        assert!(server.get("headers").is_none());
    }

    #[test]
    fn a_policy_of_none_selects_no_servers() {
        let available = vec![McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: "echo".into(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }];
        let granted = grant(
            &available,
            &McpPolicy {
                mode: PolicyMode::None,
                server_ids: vec!["echo".into()],
            },
        );
        assert!(granted.servers.is_empty());
        assert!(granted.missing.is_empty());
    }

    #[test]
    fn a_some_policy_names_the_ids_that_no_longer_exist() {
        let available = vec![McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: "echo".into(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }];
        let granted = grant(
            &available,
            &McpPolicy {
                mode: PolicyMode::Some,
                server_ids: vec!["echo".into(), "gone".into()],
            },
        );
        assert_eq!(granted.servers.len(), 1);
        assert_eq!(granted.missing, ["gone"]);
    }

    #[tokio::test]
    async fn a_duplex_server_lists_its_tool_and_a_scripted_call_reaches_it() {
        let (client_to_server, server_from_client) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            Echo.serve(server_from_client)
                .await
                .expect("echo server starts")
                .waiting()
                .await
                .expect("echo server ends cleanly");
        });

        let client = toad_client()
            .serve(client_to_server)
            .await
            .expect("client handshake");
        let listed = client.list_all_tools().await.expect("tools listed");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name.as_ref(), "shout");

        let tool = McpTool::new(
            "echo",
            "Echo",
            listed.into_iter().next().unwrap(),
            client.peer().clone(),
            Watch::new("duplex-echo"),
        );
        assert_eq!(tool.name, "echo__shout");
        let shouted = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect("the call reached the server");
        assert_eq!(shouted, "HARBOUR");

        client.cancel().await.ok();
        let _ = server.await;
    }

    #[tokio::test]
    async fn a_server_dropped_after_the_first_call_marks_its_rows_absent_once() {
        use crate::contract::ToolState;
        let persona_id = "mcp-gone-duplex";
        publish_echo(persona_id);
        let (server, client, tool) = echo_on_duplex(persona_id).await;
        let shouted = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect("the first call reached the server");
        assert_eq!(shouted, "HARBOUR");
        assert_eq!(
            echo_row(persona_id, "echo__shout").state,
            ToolState::Verified
        );

        drop(client);
        server.abort();

        let err = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect_err("the dropped server cannot answer");
        let CallError::Transport {
            notice: Some(text), ..
        } = &err
        else {
            panic!("a dead transport is a transport error, not {err:?}");
        };
        assert!(text.starts_with("The Echo MCP server went away:"), "{text}");
        assert!(
            text.contains("Its tools are gone until the teammate restarts."),
            "{text}"
        );

        let shout = echo_row(persona_id, "echo__shout");
        assert_eq!(shout.state, ToolState::Absent);
        assert!(!shout.reason.is_empty());
        assert_eq!(
            echo_row(persona_id, "echo__whisper").state,
            ToolState::Absent
        );
        assert_eq!(echo_row(persona_id, "read").state, ToolState::Verified);

        let err = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect_err("still gone");
        assert!(
            matches!(err, CallError::Transport { notice: None, .. }),
            "the notice lands once: {err:?}"
        );
    }

    #[tokio::test]
    async fn a_tool_level_error_leaves_the_ledger_verified() {
        use crate::contract::ToolState;
        let persona_id = "mcp-tool-error";
        publish_echo(persona_id);
        let (server, client, tool) = echo_on_duplex(persona_id).await;

        let err = tool
            .call(json!({ "text": "fail" }))
            .await
            .expect_err("the tool refused");
        assert!(
            matches!(&err, CallError::Tool(message) if message.contains("refused")),
            "{err:?}"
        );
        assert_eq!(
            echo_row(persona_id, "echo__shout").state,
            ToolState::Verified
        );

        let err = tool
            .call(json!({ "text": "rpc" }))
            .await
            .expect_err("the server rejected the call");
        assert!(matches!(err, CallError::Tool(_)), "{err:?}");
        assert_eq!(
            echo_row(persona_id, "echo__shout").state,
            ToolState::Verified
        );

        client.cancel().await.ok();
        let _ = server.await;
    }

    #[tokio::test]
    async fn oauth_and_static_http_are_refused_with_a_sentence() {
        let oauth = McpServer {
            id: "oauth".into(),
            name: "OAuth".into(),
            transport: McpTransport::Http {
                url: "https://example.test".into(),
                auth: HttpAuth::Oauth,
            },
            refuse: None,
        };
        let static_header = McpServer {
            id: "static".into(),
            name: "Static".into(),
            transport: McpTransport::Http {
                url: "https://example.test".into(),
                auth: HttpAuth::Static {
                    header_names: vec!["Authorization".into()],
                },
            },
            refuse: None,
        };
        let connected = connect("oauth-refuse", &[oauth, static_header]).await;
        assert!(connected.tools.is_empty());
        assert_eq!(connected.failed.len(), 2);
        assert!(connected.failed[0].reason.contains("OAuth"));
        assert!(connected.failed[1].reason.contains("Static-header"));
    }

    #[test]
    fn a_non_string_env_value_refuses_the_server_and_names_the_key() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": "/bin/true",
                "env": { "API_TOKEN": 1, "OTHER": "ok" },
            }]),
        );
        let listed = servers(&settings);
        assert_eq!(listed.len(), 1);
        let reason = unsupported(&listed[0]).expect("the server should be refused");
        assert!(
            reason.contains("API_TOKEN"),
            "the refusal did not name the key: {reason}"
        );
    }

    #[test]
    fn a_non_object_env_refuses_the_server() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": "/bin/true",
                "env": ["A=b"],
            }]),
        );
        let listed = servers(&settings);
        assert_eq!(listed.len(), 1);
        let reason = unsupported(&listed[0]).expect("the server should be refused");
        assert_eq!(
            reason,
            "env is not a map of strings; the server was not started."
        );
    }

    #[tokio::test]
    async fn a_non_string_env_value_is_an_absent_row_not_a_started_server() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": "/bin/true",
                "env": { "API_TOKEN": 1 },
            }]),
        );
        let listed = servers(&settings);
        let connected = connect("needs-token", &listed).await;
        assert!(connected.tools.is_empty());
        assert_eq!(connected.failed.len(), 1);
        assert_eq!(connected.failed[0].id, "needs-token");
        assert!(
            connected.failed[0].reason.contains("API_TOKEN"),
            "{}",
            connected.failed[0].reason
        );
    }

    #[tokio::test]
    async fn a_server_that_fails_to_start_is_a_failed_row() {
        let missing = McpServer {
            id: "gone".into(),
            name: "Gone".into(),
            transport: McpTransport::Stdio {
                command: format!(
                    "/no-such-toad-mcp-server-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        };
        let connected = connect("gone", &[missing]).await;
        assert!(connected.tools.is_empty());
        assert_eq!(connected.failed.len(), 1);
        assert!(!connected.failed[0].reason.trim().is_empty());
    }
}
