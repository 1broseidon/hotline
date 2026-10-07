//! Firecrawl's v2 search API. Keyless against the hosted endpoint; a key goes
//! as a bearer header.

use super::when::Window;
use super::{Failure, Hit, Request, Searcher, mcp};
use async_trait::async_trait;
use chrono::NaiveDate;
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
    /// News results say when: "2 hours ago", or a date.
    #[serde(default)]
    date: Option<String>,
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
    if let Some(window) = request.window {
        body["sources"] = json!(["web", "news"]);
        body["tbs"] = json!(tbs(window, request.today));
    }
    body
}

/// Firecrawl's `tbs` for a window: Google's `qdr` (past day, week, month,
/// year) when the window runs up to now, an explicit `cdr` range when it is
/// in the past.
pub(super) fn tbs(window: Window, today: NaiveDate) -> String {
    if (today - window.to).num_days() <= 1 {
        let days = (today - window.from).num_days();
        let span = match days {
            ..=1 => "d",
            2..=7 => "w",
            8..=31 => "m",
            _ => "y",
        };
        return format!("qdr:{span}");
    }
    let us = |d: NaiveDate| d.format("%-m/%-d/%Y").to_string();
    format!("cdr:1,cd_min:{},cd_max:{}", us(window.from), us(window.to))
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
        let wait = mcp::retry_after(&response);
        let raw = mcp::read_capped(response, "firecrawl").await?;
        if !status.is_success() {
            return Err(
                Failure::status("firecrawl", status.as_u16(), &raw, key.is_some()).after(wait),
            );
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
                published: raw.date.filter(|date| !date.trim().is_empty()),
            })
            .collect())
    }
}
