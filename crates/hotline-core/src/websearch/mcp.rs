//! One session-less MCP `tools/call` over HTTP, as the hosted search servers
//! take it.
//!
//! Parallel and Exa each answer a single JSON-RPC `tools/call` POST
//! with either a JSON body or one SSE frame, with no `initialize` handshake
//! first. That is simpler and faster than a full rmcp session for one call
//! and it is what ketch does against the same endpoints, so it is the port.

use super::Failure;
use serde::Deserialize;
use serde_json::{Value, json};

/// The most of a response read. A search answer is kilobytes; this is a stop
/// against a server that never ends.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
struct Rpc {
    #[serde(default)]
    result: RpcResult,
    error: Option<RpcError>,
}

#[derive(Default, Deserialize)]
struct RpcResult {
    #[serde(default)]
    content: Vec<Block>,
    #[serde(default, rename = "isError")]
    is_error: bool,
}

#[derive(Deserialize)]
struct Block {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// What a tool call answered: its text blocks, or why it failed.
pub(super) struct Answer {
    pub(super) texts: Vec<String>,
}

/// Calls `tool` on the MCP server at `endpoint`. `bearer` is sent as an
/// `Authorization` header and never placed in the URL by this function.
pub(super) async fn call(
    client: &reqwest::Client,
    name: &str,
    endpoint: &str,
    bearer: Option<&str>,
    keyed: bool,
    tool: &str,
    arguments: Value,
) -> Result<Answer, Failure> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let mut request = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(body.to_string());
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    let response = request
        .send()
        .await
        .map_err(|error| Failure::transport(name, &error))?;
    let status = response.status();
    let wait = retry_after(&response);
    let raw = read_capped(response, name).await?;
    if !status.is_success() {
        return Err(Failure::status(name, status.as_u16(), &raw, keyed).after(wait));
    }
    decode(name, &raw)
}

/// The `Retry-After` header in seconds, when there is one.
pub(super) fn retry_after(response: &reqwest::Response) -> Option<std::time::Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

pub(super) async fn read_capped(
    mut response: reqwest::Response,
    name: &str,
) -> Result<String, Failure> {
    let mut raw = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| Failure::transport(name, &error))?
    {
        raw.extend_from_slice(&chunk);
        if raw.len() > MAX_RESPONSE_BYTES {
            raw.truncate(MAX_RESPONSE_BYTES);
            break;
        }
    }
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// The payload of a response that is either one JSON document or SSE frames:
/// the last non-empty `data:` line, as ketch reads it.
fn payload(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return Some(trimmed);
    }
    raw.lines()
        .filter_map(|line| line.trim().strip_prefix("data:"))
        .map(str::trim)
        .rfind(|data| !data.is_empty())
}

/// Turns an MCP response into text blocks, failing loud on either error shape.
/// A server reports failure under HTTP 200, as a JSON-RPC error or a tool
/// `isError`; reading either as an empty success would stop the chain from
/// falling through to the next provider.
fn decode(name: &str, raw: &str) -> Result<Answer, Failure> {
    let Some(payload) = payload(raw) else {
        return Err(Failure::new(format!(
            "{name} response contained no data payload"
        )));
    };
    let rpc: Rpc = serde_json::from_str(payload)
        .map_err(|_| Failure::new(format!("failed to decode {name} response")))?;
    if let Some(error) = rpc.error {
        return Err(Failure::new(format!(
            "{name} JSON-RPC error {}: {}",
            error.code,
            Failure::detail(&error.message)
        )));
    }
    if rpc.result.is_error {
        let detail = rpc
            .result
            .content
            .iter()
            .map(|block| block.text.trim())
            .find(|text| !text.is_empty())
            .unwrap_or("");
        let failure = if detail.is_empty() {
            Failure::new(format!("{name} search tool returned an error"))
        } else {
            Failure::new(format!(
                "{name} search tool returned an error: {}",
                Failure::detail(detail)
            ))
        };
        return Err(failure);
    }
    Ok(Answer {
        texts: rpc
            .result
            .content
            .into_iter()
            .filter(|block| block.kind == "text")
            .map(|block| block.text)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_json_body_and_an_sse_frame_read_the_same() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"hi"}]}}"#;
        let sse = format!("event: message\ndata: \n\ndata: {body}\n\n");
        for raw in [body.to_string(), sse] {
            assert_eq!(decode("x", &raw).unwrap().texts, vec!["hi".to_string()]);
        }
    }

    #[test]
    fn both_error_shapes_surface_under_http_200() {
        let rpc = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"internal error"}}"#;
        assert_eq!(
            decode("exa", rpc).err().unwrap().message,
            "exa JSON-RPC error -32603: internal error"
        );
        let tool =
            r#"{"result":{"isError":true,"content":[{"type":"text","text":"quota exceeded"}]}}"#;
        assert_eq!(
            decode("exa", tool).err().unwrap().message,
            "exa search tool returned an error: quota exceeded"
        );
        let bare = r#"{"result":{"isError":true,"content":[]}}"#;
        assert_eq!(
            decode("exa", bare).err().unwrap().message,
            "exa search tool returned an error"
        );
        assert!(
            decode("exa", r#"{"result":"#)
                .err()
                .unwrap()
                .message
                .contains("failed to decode exa response")
        );
        assert!(decode("exa", "").is_err());
    }
}
