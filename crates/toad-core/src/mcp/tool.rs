//! One Rig tool per MCP tool.
//!
//! The name is prefixed with the server id so two servers that both expose
//! `search` do not collide. Description and input schema pass through as the
//! server listed them. A call is forwarded, and the result's text content
//! is what the model sees; a transport failure or an `isError` result is
//! `Err`.

use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use rmcp::RoleClient;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::Peer;
use serde_json::Value;

/// An MCP tool as Toad Agent sees it: a unique name, the server's own
/// description and schema, and a call that goes back to that server.
#[derive(Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// The server this tool came from, as the ledger names it.
    pub origin: String,
    remote_name: String,
    peer: Peer<RoleClient>,
}

impl McpTool {
    pub(crate) fn new(
        server_id: &str,
        definition: rmcp::model::Tool,
        peer: Peer<RoleClient>,
    ) -> Self {
        let remote_name = definition.name.to_string();
        Self {
            name: format!("{server_id}__{remote_name}"),
            description: definition.description.as_deref().unwrap_or("").to_string(),
            parameters: definition.schema_as_json_value(),
            origin: server_id.to_string(),
            remote_name,
            peer,
        }
    }

    /// Forward the call and return the result's text. Errors as `Err`.
    pub async fn call(&self, arguments: Value) -> Result<String, String> {
        let arguments = match arguments {
            Value::Null => None,
            Value::Object(object) => Some(object),
            other => {
                return Err(format!(
                    "MCP tool '{}' expected a JSON object, got {other}",
                    self.name
                ));
            }
        };
        let result = self
            .peer
            .call_tool(CallToolRequestParams {
                meta: None,
                name: self.remote_name.clone().into(),
                arguments,
                task: None,
            })
            .await
            .map_err(|error| format!("MCP tool '{}' request failed: {error}", self.name))?;
        let text = result_text(&result);
        if result.is_error == Some(true) {
            Err(if text.is_empty() {
                format!("MCP tool '{}' reported an error", self.name)
            } else {
                text
            })
        } else {
            Ok(text)
        }
    }

    /// The same tool, registered on a Rig agent.
    pub fn as_dynamic(&self) -> DynamicTool {
        let tool = self.clone();
        DynamicTool::new(
            self.name.clone(),
            self.description.clone(),
            self.parameters.clone(),
            move |_context, arguments| {
                let tool = tool.clone();
                Box::pin(async move {
                    match tool.call(arguments).await {
                        Ok(text) => Ok(ToolOutput::text(text)),
                        Err(error) => Err(ToolExecutionError::other(error)),
                    }
                })
            },
        )
    }
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
