//! The running computer's guide, fetched from the computer itself.
//!
//! Hotline Computer bundles a skill for the release it is and serves it from
//! `state {"action":"guide"}` with its version and a checksum. Hotline never
//! bundles a copy: the desk pins one image tag, but an existing container
//! keeps the release it was made on (see docs/computer.md), so the only
//! guide guaranteed to describe the tools the agent actually has is the one
//! the running computer hands over. It is asked for once, when the computer
//! is granted at session start, and written into the workspace as the
//! `hotline-computer` skill. A computer that cannot answer — an older image
//! without the action, an endpoint that will not open — leaves no skill, and
//! the preamble tells the agent to ask the computer itself.

use super::Ready;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Implementation};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};
use std::time::Duration;

/// A computer that is healthy answers its own guide in well under this;
/// one that does not is not holding up the teammate's start for longer.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// What a running computer says its guide is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guide {
    pub version: String,
    pub sha256: String,
    pub skill: String,
}

/// Asks the computer at `ready` for its guide.
pub async fn fetch(ready: &Ready) -> Result<Guide, String> {
    tokio::time::timeout(FETCH_TIMEOUT, ask(ready))
        .await
        .map_err(|_| "The computer did not answer for its guide in time.".to_string())?
}

async fn ask(ready: &Ready) -> Result<Guide, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let transport = StreamableHttpClientTransport::with_client(
        client,
        StreamableHttpClientTransportConfig::with_uri(ready.url.clone())
            .auth_header(ready.token.clone()),
    );
    let service = ClientInfo::new(
        Default::default(),
        Implementation::new("hotline", env!("CARGO_PKG_VERSION")),
    )
    .serve(transport)
    .await
    .map_err(|error| format!("The computer's endpoint did not open: {error}"))?;
    let answered = service
        .call_tool(
            CallToolRequestParams::new("state").with_arguments(
                json!({"action": "guide"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await;
    service.cancel().await.ok();
    let result = answered.map_err(|error| format!("The computer refused its guide: {error}"))?;
    let text = result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .unwrap_or_default();
    if result.is_error == Some(true) {
        return Err(format!("The computer refused its guide: {text}"));
    }
    parse(&text)
}

fn parse(text: &str) -> Result<Guide, String> {
    let manifest: Value = serde_json::from_str(text)
        .map_err(|_| "The computer's guide was not the manifest Hotline expects.".to_string())?;
    let field = |name: &str| {
        manifest
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("The computer's guide has no {name}."))
    };
    Ok(Guide {
        version: field("version")?,
        sha256: field("sha256")?,
        skill: field("skill")?,
    })
}

/// A computer a test can grant: `/health` open and `state guide` answering
/// the manifest a real one does, on a loopback port the fake runtime reports.
#[cfg(test)]
pub(crate) mod fake {
    use rmcp::ErrorData;
    use rmcp::handler::server::ServerHandler;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    };
    use rmcp::service::RequestContext;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// The skill text a release `version` would serve, with the placeholder
    /// filled the way Hotline Computer's own build fills it.
    pub(crate) fn skill_of(version: &str) -> String {
        format!(
            "---\nname: hotline-computer\ndescription: Operate a Hotline Computer through its MCP tools.\n---\n\nThis guide ships with Hotline Computer {version}.\n"
        )
    }

    #[derive(Clone)]
    struct FakeComputer {
        /// The release this fake is; `None` is an older image whose `state`
        /// has no `guide` action.
        version: Option<String>,
    }

    impl ServerHandler for FakeComputer {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            let schema = json!({"type":"object","properties":{"action":{"type":"string"}}});
            Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "state",
                "Computer state.",
                Arc::new(schema.as_object().cloned().unwrap()),
            )]))
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            context: RequestContext<rmcp::RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            let bearer = context
                .extensions
                .get::<axum::http::request::Parts>()
                .and_then(|parts| parts.headers.get(axum::http::header::AUTHORIZATION))
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or("");
            let action = request
                .arguments
                .as_ref()
                .and_then(|arguments| arguments.get("action"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let answered = match (&self.version, action) {
                _ if bearer.is_empty() => {
                    CallToolResult::error(vec![ContentBlock::text("no bearer token")])
                }
                (Some(version), "guide") => {
                    let skill = skill_of(version);
                    let manifest = json!({
                        "version": version,
                        "build": {"channel": "release", "revision": "fixture"},
                        "sha256": format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(skill.as_bytes())),
                        "skill": skill,
                        "catalog": {},
                    });
                    CallToolResult::success(vec![ContentBlock::text(manifest.to_string())])
                }
                _ => CallToolResult::error(vec![ContentBlock::text(format!(
                    "state: unknown action {action}"
                ))]),
            };
            Ok(answered.into())
        }
    }

    /// Serves a computer on a loopback port and answers the port. `version`
    /// is the release it claims; `None` is an image too old to have a guide.
    pub(crate) async fn serve(version: Option<&str>) -> u16 {
        let computer = FakeComputer {
            version: version.map(str::to_owned),
        };
        let service: StreamableHttpService<FakeComputer, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(computer.clone()),
                Arc::new(LocalSessionManager::default()),
                Default::default(),
            );
        let app = axum::Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        port
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_guide_is_the_release_the_computer_says_it_is() {
        let port = fake::serve(Some("0.9.1")).await;
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "fixture-token".into(),
        };
        let guide = fetch(&ready).await.expect("the fake answers");
        assert_eq!(guide.version, "0.9.1");
        assert_eq!(guide.skill, fake::skill_of("0.9.1"));
        assert_eq!(guide.sha256.len(), 64);
    }

    #[tokio::test]
    async fn an_older_image_without_the_action_is_a_sentence_not_a_skill() {
        let port = fake::serve(None).await;
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "fixture-token".into(),
        };
        let refused = fetch(&ready).await.unwrap_err();
        assert!(refused.contains("unknown action guide"), "{refused}");
    }

    #[test]
    fn a_manifest_missing_its_skill_is_refused_by_name() {
        let refused = parse(r#"{"version":"0.9.1","sha256":"abc"}"#).unwrap_err();
        assert_eq!(refused, "The computer's guide has no skill.");
        assert!(parse("not json").is_err());
    }
}
