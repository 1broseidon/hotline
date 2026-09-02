//! A one-tool stdio MCP server the tests spawn. It exists so a persona's
//! `mcpServers` entry can name a real command, which is the path Toad Agent
//! actually connects.

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServiceExt};
use serde_json::{Value, json};
use std::future::Future;
use std::sync::Arc;

struct Echo;

fn shout_schema() -> Arc<JsonObject> {
    let schema = json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"],
    });
    Arc::new(schema.as_object().unwrap().clone())
}

impl ServerHandler for Echo {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: vec![Tool::new(
                "shout",
                "Echo the text back in upper case.",
                shout_schema(),
            )],
            ..Default::default()
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, rmcp::ErrorData>> + Send + '_ {
        let text = request
            .arguments
            .as_ref()
            .and_then(|args| args.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_uppercase();
        std::future::ready(Ok(CallToolResult::success(vec![Content::text(text)])))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Echo.serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
