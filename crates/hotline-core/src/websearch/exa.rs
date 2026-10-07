//! Exa's hosted MCP: `web_search_exa`. Keyless; a key rides in the endpoint's
//! `exaApiKey` query parameter, which is how Exa takes it. That puts the key
//! in a URL, so no error here is built from the URL: see [`Failure::transport`].

use super::{Failure, Hit, Searcher, mcp};
use async_trait::async_trait;
use serde_json::json;

pub const ENDPOINT: &str = "https://mcp.exa.ai/mcp";

pub struct Exa {
    client: reqwest::Client,
    endpoint: String,
    key: Option<String>,
}

impl Exa {
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

    fn url(&self) -> String {
        match self
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            Some(key) => match url::Url::parse(&self.endpoint) {
                Ok(mut url) => {
                    url.query_pairs_mut().append_pair("exaApiKey", key);
                    url.to_string()
                }
                Err(_) => self.endpoint.clone(),
            },
            None => self.endpoint.clone(),
        }
    }
}

#[async_trait]
impl Searcher for Exa {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, Failure> {
        let answer = mcp::call(
            &self.client,
            "exa",
            &self.url(),
            None,
            self.key.is_some(),
            "web_search_exa",
            json!({
                "query": query,
                "numResults": limit,
                "type": "auto",
                "livecrawl": "fallback",
                "contextMaxCharacters": 3000,
            }),
        )
        .await?;
        // No content is a real "nothing matched", not a failure.
        let mut hits = Vec::new();
        for text in &answer.texts {
            if hits.len() >= limit {
                break;
            }
            hits.extend(parse_content(text, limit - hits.len()));
        }
        Ok(hits)
    }
}

/// Labels Exa emits as metadata rather than content.
fn known_prefix(line: &str) -> bool {
    [
        "Title:",
        "URL:",
        "Highlights:",
        "Published date:",
        "Author:",
        "Score:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

/// Exa's text-formatted results: blocks separated by `---`, each with `Title:`
/// and `URL:` lines followed by highlight text, the first plain line of which
/// is the snippet.
pub(super) fn parse_content(raw: &str, limit: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    for block in raw.split("\n---\n") {
        if hits.len() >= limit {
            break;
        }
        let mut hit = Hit::default();
        for line in block.lines().map(str::trim) {
            if let Some(title) = line.strip_prefix("Title:") {
                hit.title = title.trim().to_string();
            } else if let Some(url) = line.strip_prefix("URL:") {
                hit.url = url.trim().to_string();
            } else if !line.is_empty() && !known_prefix(line) && hit.snippet.is_empty() {
                hit.snippet = line.to_string();
            }
        }
        if !hit.title.is_empty() && !hit.url.is_empty() {
            hits.push(hit);
        }
    }
    hits
}
