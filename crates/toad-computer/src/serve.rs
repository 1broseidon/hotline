use std::sync::Arc;

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::ErrorData;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};

use crate::{App, Config, tools};

const MAX_REQUEST_BODY: usize = 50 * 1024 * 1024 * 4 / 3 + 1024 * 1024;

#[derive(Clone)]
struct ComputerTools {
    app: App,
}

impl ServerHandler for ComputerTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("toad-computer", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::descriptors(
            &self.app.config.home.to_string_lossy(),
        )))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let holder = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.headers.get("X-Computer-Holder"))
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .unwrap_or("anonymous")
            .to_owned();
        let arguments = request.arguments.map_or(Value::Null, Value::Object);
        let result = match tools::call(&self.app, &request.name, arguments, &holder).await {
            Ok(content) => CallToolResult::success(content),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(error)]),
        };
        Ok(result.into())
    }
}

pub async fn run(config: Config) -> Result<(), String> {
    tokio::fs::create_dir_all(&config.home)
        .await
        .map_err(|error| format!("create {}: {error}", config.home.display()))?;
    let address = config.addr.clone();
    let expected_token = config.token.clone();
    let app = App::new(config);
    let service: StreamableHttpService<ComputerTools, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(ComputerTools { app: app.clone() }),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default()
                .disable_allowed_hosts()
                .with_max_request_body_bytes(MAX_REQUEST_BODY),
        );
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(
            move |request: Request, next: Next| {
                let expected_token = expected_token.clone();
                async move { authenticate(request, next, expected_token.as_deref()).await }
            },
        ));
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .map_err(|error| format!("bind {address}: {error}"))?;
    eprintln!("toad-computer listening on {address}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|error| error.to_string())
}

async fn authenticate(request: Request, next: Next, token: Option<&str>) -> Response {
    if request.uri().path() == "/health" || token.is_none() {
        return next.run(request).await;
    }
    let expected = format!("Bearer {}", token.expect("checked above"));
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if same_secret(presented.as_bytes(), expected.as_bytes()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        )
            .into_response()
    }
}

fn same_secret(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_comparison_needs_equal_bytes() {
        assert!(same_secret(b"Bearer token", b"Bearer token"));
        assert!(!same_secret(b"Bearer token", b"Bearer other"));
        assert!(!same_secret(b"", b"Bearer token"));
    }
}
