//! Parallel's hosted Search MCP: `web_search` with an objective and queries.
//! Keyless; a key, when there is one, goes as a bearer header for the higher
//! limits Parallel gives a keyed caller.

use super::{Failure, Hit, Request, Searcher, bounded, mcp, one_line};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub const ENDPOINT: &str = "https://search.parallel.ai/mcp";
const DESCRIPTION_MAX_CHARS: usize = 300;

pub struct Parallel {
    client: reqwest::Client,
    endpoint: String,
    key: Option<String>,
}

impl Parallel {
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
    results: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    excerpts: Vec<String>,
    #[serde(default)]
    publish_date: Option<String>,
}

#[async_trait]
impl Searcher for Parallel {
    /// Parallel has no result-limit argument, so the depth is applied here. It has
    /// no language or date field either, but `objective` is a sentence it reads,
    /// so a language or a wish for recent news goes there.
    async fn search(&self, request: &Request) -> Result<Vec<Hit>, Failure> {
        let limit = request.depth;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let answer = mcp::call(
            &self.client,
            "parallel",
            &self.endpoint,
            self.key.as_deref(),
            self.key.is_some(),
            "web_search",
            json!({ "objective": request.objective(), "search_queries": [request.query] }),
        )
        .await?;
        let mut hits = Vec::new();
        let mut found_text = false;
        for text in answer.texts.iter().filter(|text| !text.trim().is_empty()) {
            found_text = true;
            let payload: Payload = serde_json::from_str(text)
                .map_err(|_| Failure::new("failed to decode parallel search results"))?;
            for raw in payload.results {
                if hits.len() >= limit {
                    break;
                }
                if raw.title.trim().is_empty() || raw.url.trim().is_empty() {
                    continue;
                }
                let excerpt = raw
                    .excerpts
                    .iter()
                    .map(|excerpt| excerpt.trim())
                    .find(|excerpt| !excerpt.is_empty())
                    .unwrap_or("");
                hits.push(Hit {
                    title: one_line(&raw.title),
                    url: raw.url,
                    snippet: bounded(&one_line(excerpt), DESCRIPTION_MAX_CHARS),
                    published: raw.publish_date.filter(|at| !at.trim().is_empty()),
                });
            }
            if hits.len() >= limit {
                break;
            }
        }
        if !found_text {
            return Err(Failure::new("parallel response contained no text results"));
        }
        Ok(hits)
    }
}
