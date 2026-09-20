//! Handing a running computer a saved login, out of band.
//!
//! The computer already knows how to load cookies into its browser: the agent
//! calls `state login_load`, which reads `.hotline/logins/<name>.json` and
//! restores it. Cookie import reuses that, driven by the desk instead of the
//! agent — the operator picked the sites, so the desk uploads the file through
//! the viewer's file endpoint and calls the load itself. The agent takes no
//! step, and there is no new tool for it to call. The cookie values ride from
//! the desk to the container over its authenticated loopback port and are
//! written to a file inside the sandbox the operator granted; they never enter
//! the tape, the model, or a log.

use super::Ready;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Implementation};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Uploads the login and loads it into the computer's browser. `saved` is a
/// `SavedLogin` document as the container writes them: `{name, browser,
/// created_at, cookies, storage}`.
pub async fn deliver(ready: &Ready, name: &str, saved: &Value) -> Result<(), String> {
    upload(ready, name, saved).await?;
    load(ready, name).await
}

/// Writes the document to `.hotline/logins/<name>.json` through the viewer's
/// authenticated `POST /files`, the same endpoint the operator's file drops
/// use. The bearer rides as a query parameter because the browser that
/// endpoint was built for cannot set a header; the desk matches it here.
async fn upload(ready: &Ready, name: &str, saved: &Value) -> Result<(), String> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let path = format!(".hotline/logins/{name}.json");
    let url = reqwest::Url::parse_with_params(
        &format!("{base}/files"),
        &[("token", ready.token.as_str()), ("path", path.as_str())],
    )
    .map_err(|error| format!("Could not address the computer's file endpoint: {error}"))?;
    let body = serde_json::to_vec(saved).map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(url)
        .body(body)
        .send()
        .await
        .map_err(|error| format!("Could not send the cookies to the computer: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "The computer refused the cookies ({}).",
            response.status()
        ));
    }
    Ok(())
}

/// Calls `state login_load` on the computer's MCP endpoint, the same way the
/// desk fetches the guide, so the browser picks the cookies up now.
async fn load(ready: &Ready, name: &str) -> Result<(), String> {
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?,
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
                json!({"action": "login_load", "name": name})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await;
    service.cancel().await.ok();
    let result =
        answered.map_err(|error| format!("The computer could not load the cookies: {error}"))?;
    if result.is_error == Some(true) {
        let text = result
            .content
            .iter()
            .find_map(|block| block.as_text().map(|text| text.text.clone()))
            .unwrap_or_default();
        return Err(format!("The computer could not load the cookies: {text}"));
    }
    Ok(())
}
