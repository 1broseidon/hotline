//! What a teammate takes off its computer to send the person: a file, over
//! the computer's own download door, or the screen as it is now, from its
//! `capture` tool (BRO-98).

use super::{HOME_MOUNT, Ready};
use base64::{Engine, prelude::BASE64_STANDARD};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Implementation};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};
use std::time::Duration;

/// A local container hands over the largest file a teammate may send in
/// well under this.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// A screenshot waits for the screen to settle, and then encodes a PNG.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(20);

/// A path the teammate named on its computer, made absolute: `~` and a
/// relative path are both the home, where the computer's own tools start.
/// The computer checks the path itself, and hands over nothing outside the
/// home.
pub(crate) fn on_computer(requested: &str) -> String {
    let requested = requested.trim();
    if requested == "~" {
        return HOME_MOUNT.to_string();
    }
    if let Some(rest) = requested.strip_prefix("~/") {
        return format!("{HOME_MOUNT}/{rest}");
    }
    if requested.starts_with('/') {
        return requested.to_string();
    }
    format!("{HOME_MOUNT}/{requested}")
}

/// The file at `path` on the computer, whole, or a sentence saying why not.
/// A file over `limit` is refused as soon as its size is known, before any
/// more of it is read. A release too old to hand a file over this way is
/// said as the pane's Update: one from before 0.6 has no door and answers
/// 404, and 0.6's door took the token only in the address and answers 401.
pub(crate) async fn download(
    ready: &Ready,
    path: &str,
    limit: u64,
    too_large: impl Fn(Option<u64>) -> String,
) -> Result<Vec<u8>, String> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let mut url = reqwest::Url::parse(&format!("{base}/files/download"))
        .map_err(|error| format!("The computer's address is not usable: {error}"))?;
    url.query_pairs_mut().append_pair("path", path);
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(url)
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| format!("The computer did not answer: {error}"))?;
    if matches!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::UNAUTHORIZED
    ) {
        return Err(
            "Your computer's release cannot hand over files. The person can update it from your pane."
                .to_string(),
        );
    }
    if !response.status().is_success() {
        let status = response.status();
        let reason = response
            .text()
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
            .and_then(|body| body["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| status.to_string());
        return Err(format!(
            "The computer would not hand over {path}: {reason}."
        ));
    }
    if let Some(length) = response.content_length()
        && length > limit
    {
        return Err(too_large(Some(length)));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("{path} did not arrive whole: {error}"))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > limit {
            return Err(too_large(None));
        }
    }
    Ok(bytes)
}

/// The computer's screen now, or one window of it, or one region, as the
/// PNG its `capture` tool takes. Scaled as the tool scales every picture,
/// to at most 1568 px on the longer edge.
pub(crate) async fn screenshot(
    ready: &Ready,
    window: Option<&str>,
    region: Option<[i64; 4]>,
) -> Result<Vec<u8>, String> {
    let mut arguments = json!({"mode": "image"});
    if let Some(window) = window {
        arguments["window"] = json!(window);
    }
    if let Some(region) = region {
        arguments["region"] = json!(region);
    }
    let answered = tokio::time::timeout(CAPTURE_TIMEOUT, capture(ready, arguments))
        .await
        .map_err(|_| "The computer did not take the screenshot in time.".to_string())??;
    BASE64_STANDARD
        .decode(answered)
        .map_err(|_| "The computer's screenshot was not a picture.".to_string())
}

/// Calls `capture` and answers the base64 of the picture it returned.
async fn capture(ready: &Ready, arguments: Value) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(CAPTURE_TIMEOUT)
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
            CallToolRequestParams::new("capture")
                .with_arguments(arguments.as_object().cloned().unwrap_or_default()),
        )
        .await;
    service.cancel().await.ok();
    let result =
        answered.map_err(|error| format!("The computer could not take the screenshot: {error}"))?;
    if result.is_error == Some(true) {
        let text = result
            .content
            .iter()
            .find_map(|block| block.as_text().map(|text| text.text.clone()))
            .unwrap_or_default();
        // A release from before 0.10 has no `image` mode, and says so in
        // words that release will never change.
        if text.contains(r#"unknown action "image""#) {
            return Err(
                "Your computer's release cannot take a screenshot to send. The person can update it from your pane."
                    .to_string(),
            );
        }
        return Err(format!(
            "The computer could not take the screenshot: {text}"
        ));
    }
    result
        .content
        .iter()
        .find_map(|block| block.as_image().map(|image| image.data.clone()))
        .ok_or_else(|| "The computer's screenshot had no picture in it.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::guide::fake;

    fn ready(port: u16) -> Ready {
        Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "fake-token".to_string(),
        }
    }

    fn over(size: Option<u64>) -> String {
        format!("too large: {size:?}")
    }

    #[test]
    fn a_path_on_the_computer_starts_at_its_home() {
        assert_eq!(on_computer("out/chart.png"), "/home/agent/out/chart.png");
        assert_eq!(on_computer("~/out/chart.png"), "/home/agent/out/chart.png");
        assert_eq!(on_computer("~"), "/home/agent");
        assert_eq!(on_computer(" /tmp/x.log "), "/tmp/x.log");
    }

    #[tokio::test]
    async fn a_file_comes_off_the_computer_whole_and_under_its_cap() {
        let (port, _) = fake::serve_taking(Some("0.10.1")).await;
        let ready = ready(port);
        let bytes = download(&ready, "/home/agent/report.txt", 1024, over)
            .await
            .unwrap();
        assert_eq!(bytes, fake::FILE_BODY);

        let refused = download(&ready, "/home/agent/report.txt", 4, over)
            .await
            .unwrap_err();
        assert_eq!(refused, over(Some(fake::FILE_BODY.len() as u64)));

        let missing = download(&ready, "/home/agent/absent.txt", 1024, over)
            .await
            .unwrap_err();
        assert_eq!(
            missing,
            "The computer would not hand over /home/agent/absent.txt: path must be under /home/agent/."
        );

        let stranger = Ready {
            token: String::new(),
            ..ready
        };
        assert!(
            download(&stranger, "/home/agent/report.txt", 1024, over)
                .await
                .is_err()
        );

        let (old, _) = fake::serve_taking(Some("0.6.0")).await;
        assert_eq!(
            download(&self::ready(old), "/home/agent/report.txt", 1024, over)
                .await
                .unwrap_err(),
            "Your computer's release cannot hand over files. The person can update it from your pane."
        );
    }

    #[tokio::test]
    async fn the_screen_comes_off_the_computer_as_the_picture_capture_took() {
        let (port, _) = fake::serve_taking(Some("0.10.1")).await;
        let ready = ready(port);
        let whole = screenshot(&ready, None, None).await.unwrap();
        assert!(whole.starts_with(b"\x89PNG"));

        let refused = screenshot(&ready, Some("Nowhere"), None).await.unwrap_err();
        assert!(refused.contains("no window is"), "{refused}");

        let (old, _) = fake::serve_taking(Some("0.9.1")).await;
        assert_eq!(
            screenshot(&self::ready(old), None, None).await.unwrap_err(),
            "Your computer's release cannot take a screenshot to send. The person can update it from your pane."
        );
    }
}
