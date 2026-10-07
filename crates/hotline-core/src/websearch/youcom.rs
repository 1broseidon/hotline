//! You.com's hosted MCP: the `you-search` tool. Keyless it uses the free
//! profile; a key switches to the authenticated server and goes as a bearer
//! header, never in the URL.

use super::{Failure, Hit, Searcher, mcp};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub const ENDPOINT: &str = "https://api.you.com/mcp";

pub struct Youcom {
    client: reqwest::Client,
    endpoint: String,
    key: Option<String>,
}

impl Youcom {
    pub fn new(client: reqwest::Client, key: Option<String>) -> Self {
        Self::at(client, ENDPOINT, key)
    }

    pub fn at(client: reqwest::Client, endpoint: impl Into<String>, key: Option<String>) -> Self {
        Self {
            client,
            endpoint: endpoint.into(),
            key,
        }
    }
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    results: Results,
}

#[derive(Default, Deserialize)]
struct Results {
    #[serde(default)]
    web: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    snippets: Vec<String>,
}

#[async_trait]
impl Searcher for Youcom {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, Failure> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let key = self
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        let endpoint = if key.is_some() {
            self.endpoint.clone()
        } else {
            format!("{}?profile=free", self.endpoint)
        };
        let answer = mcp::call(
            &self.client,
            "youcom",
            &endpoint,
            key,
            key.is_some(),
            "you-search",
            json!({ "query": query, "count": limit, "extraction": "none" }),
        )
        .await
        .map_err(|failure| {
            if failure.rejected_key {
                Failure::new("youcom: API key rejected (check it in Settings, Tools)")
            } else {
                failure
            }
        })?;
        let mut hits = Vec::new();
        let mut found_text = false;
        for text in answer.texts.iter().filter(|text| !text.trim().is_empty()) {
            found_text = true;
            let payload: Payload = serde_json::from_str(text)
                .map_err(|_| Failure::new("failed to decode youcom search results"))?;
            for raw in payload.results.web {
                if hits.len() >= limit {
                    break;
                }
                if raw.url.trim().is_empty() {
                    continue;
                }
                let snippet = if raw.description.is_empty() {
                    raw.snippets.first().cloned().unwrap_or_default()
                } else {
                    raw.description
                };
                hits.push(Hit {
                    title: raw.title,
                    url: raw.url,
                    snippet,
                });
            }
            if hits.len() >= limit {
                break;
            }
        }
        if !found_text {
            return Err(Failure::new("youcom response contained no text results"));
        }
        Ok(hits)
    }
}
