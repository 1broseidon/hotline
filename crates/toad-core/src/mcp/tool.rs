//! One MCP tool as Toad Agent calls it.
//!
//! The name is prefixed with the server id so two servers that both expose
//! `search` do not collide. Description and input schema pass through as the
//! server listed them. A call is forwarded, and the result's text content
//! is what the model sees. A transport failure or an `isError` result is
//! `Err`; only the transport case marks the origin absent, because that is
//! the server going away, not the tool answering.

use crate::contract::ToolSourceKind;
use crate::session::ledger;
use rmcp::RoleClient;
use rmcp::ServiceError;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::Peer;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

/// Why a call did not succeed. Both variants' text is what the model sees.
#[derive(Debug)]
pub enum CallError {
    /// The server answered this failure — a tool `isError` result or a
    /// JSON-RPC error — or the call failed without the origin disappearing
    /// (a timeout, a cancel, an unexpected response).
    Tool(String),
    /// The transport died. `notice` is the tape sentence the first time this
    /// origin has gone away this session.
    Transport {
        message: String,
        notice: Option<String>,
    },
}

impl CallError {
    fn message(&self) -> &str {
        match self {
            Self::Tool(message) | Self::Transport { message, .. } => message,
        }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CallError {}

/// Shared across every tool of one session, so two tools from the same
/// dead server write one notice, not one each.
pub(crate) struct Watch {
    persona_id: String,
    announced: Mutex<HashSet<String>>,
}

impl Watch {
    pub(crate) fn new(persona_id: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            persona_id: persona_id.into(),
            announced: Mutex::new(HashSet::new()),
        })
    }

    fn gone(&self, origin: &str, name: &str, reason: &str) -> Option<String> {
        ledger::mark_absent(&self.persona_id, ToolSourceKind::Mcp, origin, reason);
        let mut announced = self
            .announced
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !announced.insert(origin.to_string()) {
            return None;
        }
        Some(format!(
            "The {name} MCP server went away: {reason}. Its tools are gone until the teammate restarts."
        ))
    }
}

/// An MCP tool as Toad Agent sees it: a unique name, the server's own
/// description and schema, and a call that goes back to that server.
#[derive(Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// The server this tool came from, as the ledger names it.
    pub origin: String,
    server_name: String,
    remote_name: String,
    peer: Peer<RoleClient>,
    watch: Arc<Watch>,
}

impl McpTool {
    pub(crate) fn new(
        server_id: &str,
        server_name: &str,
        definition: rmcp::model::Tool,
        peer: Peer<RoleClient>,
        watch: Arc<Watch>,
    ) -> Self {
        let remote_name = definition.name.to_string();
        Self {
            name: format!("{server_id}__{remote_name}"),
            description: definition.description.as_deref().unwrap_or("").to_string(),
            parameters: definition.schema_as_json_value(),
            origin: server_id.to_string(),
            server_name: server_name.to_string(),
            remote_name,
            peer,
            watch,
        }
    }

    /// Forward the call and return the result's text. Errors as `Err`.
    pub async fn call(&self, arguments: Value) -> Result<String, CallError> {
        let arguments = match arguments {
            Value::Null => None,
            Value::Object(object) => Some(object),
            other => {
                return Err(CallError::Tool(format!(
                    "MCP tool '{}' expected a JSON object, got {other}",
                    self.name
                )));
            }
        };
        let mut request = CallToolRequestParams::new(self.remote_name.clone());
        request.arguments = arguments;
        let result = match self.peer.call_tool(request).await {
            Ok(result) => result,
            Err(error) => {
                let message = format!("MCP tool '{}' request failed: {error}", self.name);
                if is_transport(&error) {
                    let notice =
                        self.watch
                            .gone(&self.origin, &self.server_name, &error.to_string());
                    return Err(CallError::Transport { message, notice });
                }
                return Err(CallError::Tool(message));
            }
        };
        let text = result_text(&result);
        if result.is_error == Some(true) {
            Err(CallError::Tool(if text.is_empty() {
                format!("MCP tool '{}' reported an error", self.name)
            } else {
                text
            }))
        } else {
            Ok(text)
        }
    }
}

/// The origin is gone: the process exited, the socket closed, the HTTP
/// endpoint could not be reached. A JSON-RPC error and an `isError` result
/// are answers, and do not take this path.
fn is_transport(error: &ServiceError) -> bool {
    matches!(
        error,
        ServiceError::TransportClosed | ServiceError::TransportSend(_)
    )
}

fn result_text(result: &CallToolResult) -> String {
    let texts: Vec<String> = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect();
    if !texts.is_empty() {
        return texts.join("\n");
    }
    result
        .structured_content
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default()
}
