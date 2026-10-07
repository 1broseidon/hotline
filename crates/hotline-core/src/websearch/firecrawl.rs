//! Firecrawl's v2 search API. Keyless against the hosted endpoint; a key goes
//! as a bearer header.

use super::{Failure, Hit, Request, Searcher, mcp};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub const BASE: &str = "https://api.firecrawl.dev";

pub struct Firecrawl {
    client: reqwest::Client,
    base: String,
    key: Option<String>,
}

impl Firecrawl {
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
    data: Data,
}

#[derive(Default, Deserialize)]
struct Data {
    #[serde(default)]
    web: Vec<Raw>,
    #[serde(default)]
    news: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
    /// News results carry page text here instead.
    #[serde(default)]
    snippet: String,
}

/// The request body. Firecrawl's v2 search takes `lang`, and for the news
/// `sources` and a time bound (`tbs`, here the past week).
fn body(request: &Request) -> serde_json::Value {
    let mut body = json!({
        "query": request.query,
        "limit": request.depth,
        "integration": "_hotline",
    });
    if let Some(language) = request.language {
        body["lang"] = json!(language.code);
    }
    if request.recency {
        body["sources"] = json!(["web", "news"]);
        body["tbs"] = json!("qdr:w");
    }
    body
}

#[async_trait]
impl Searcher for Firecrawl {
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
        let mut request = self
            .client
            .post(format!("{}/v2/search", self.base.trim_end_matches('/')))
            .header("Content-Type", "application/json")
            .body(body(request).to_string());
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| Failure::transport("firecrawl", &error))?;
        let status = response.status();
        let raw = mcp::read_capped(response, "firecrawl").await?;
        if !status.is_success() {
            return Err(Failure::status(
                "firecrawl",
                status.as_u16(),
                &raw,
                key.is_some(),
            ));
        }
        let parsed: Response = serde_json::from_str(&raw)
            .map_err(|_| Failure::new("failed to decode firecrawl response"))?;
        Ok(parsed
            .data
            .web
            .into_iter()
            .chain(parsed.data.news)
            .filter(|raw| !raw.url.is_empty())
            .take(limit)
            .map(|raw| Hit {
                title: raw.title,
                url: raw.url,
                snippet: if raw.description.is_empty() {
                    raw.snippet
                } else {
                    raw.description
                },
            })
            .collect())
    }
}
