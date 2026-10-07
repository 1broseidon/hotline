//! Keenable's REST search, a web index built for agents. Keyless it posts to
//! the public endpoint, rate-limited by the hour; with a key it posts to the
//! authenticated one with `X-API-Key`.

use super::{Failure, Hit, Request, Searcher, mcp, one_line};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub const BASE: &str = "https://api.keenable.ai";

/// Keenable returns whole pages as snippets, an order of magnitude more than
/// the others; this is what is kept of one.
const SNIPPET_MAX_CHARS: usize = 500;

pub struct Keenable {
    client: reqwest::Client,
    base: String,
    key: Option<String>,
}

impl Keenable {
    pub fn new(client: reqwest::Client, key: Option<String>) -> Self {
        Self::at(client, BASE, key)
    }

    pub fn at(client: reqwest::Client, base: impl Into<String>, key: Option<String>) -> Self {
        Self {
            client,
            base: base.into(),
            key,
        }
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    /// The page text. `description` is the page's meta description, empty for
    /// most pages, so it is only a fallback.
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    published_at: Option<String>,
}

/// The request body: Keenable takes a result count and a publication date,
/// and no language, so only those are sent.
fn body(request: &Request) -> serde_json::Value {
    let mut body = json!({
        "query": request.query,
        "mode": "pro",
        "max_results": request.depth.clamp(1, 50),
    });
    if let Some(window) = request.window {
        body["published_after"] = json!(window.from.format("%Y-%m-%d").to_string());
    }
    body
}

#[async_trait]
impl Searcher for Keenable {
    async fn search(&self, request: &Request) -> Result<Vec<Hit>, Failure> {
        let limit = request.depth;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let key = self
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        let path = if key.is_some() {
            "/v1/search"
        } else {
            "/v1/search/public"
        };
        let mut request = self
            .client
            .post(format!("{}{path}", self.base))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("X-Keenable-Title", "Hotline")
            .body(body(request).to_string());
        if let Some(key) = key {
            request = request.header("X-API-Key", key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| Failure::transport("keenable", &error))?;
        let status = response.status();
        let wait = mcp::retry_after(&response);
        let raw = mcp::read_capped(response, "keenable").await?;
        if !status.is_success() {
            return Err(
                Failure::status("keenable", status.as_u16(), &raw, key.is_some()).after(wait),
            );
        }
        let parsed: Response = serde_json::from_str(&raw)
            .map_err(|_| Failure::new("failed to decode keenable response"))?;
        Ok(parsed
            .results
            .into_iter()
            .take(limit)
            .map(|raw| {
                // The description is a short summary; the snippet is page text,
                // chrome and all, so it is the fallback.
                let text = if raw.description.trim().is_empty() {
                    raw.snippet
                } else {
                    raw.description
                };
                Hit {
                    title: raw.title,
                    url: raw.url,
                    snippet: one_line(&text).chars().take(SNIPPET_MAX_CHARS).collect(),
                    published: raw.published_at.filter(|at| !at.trim().is_empty()),
                }
            })
            .collect())
    }
}
