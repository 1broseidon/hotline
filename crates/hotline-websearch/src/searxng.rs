//! A SearXNG instance the person runs or chose: `GET <instance>/search?q=…&format=json`.
//!
//! Unlike the hosted providers it has no address of its own. The desk holds
//! the instance's URL ([`instance_url`] is what it may be), and the provider
//! is off until the person switches it on with one set. SearXNG has no key of
//! its own; an instance that wants one sits behind a gateway, so a saved key
//! goes as `Authorization: Bearer` (oauth2-proxy, a JWT or token gateway, a
//! reverse proxy's bearer rule) and as `X-API-Key` (the header LibreChat's
//! SearXNG client sends, so an instance set up for it works unchanged). Never
//! in the URL, where an access log would keep it.
//!
//! The query and the key go only to the host the person named: a redirect is
//! followed only while it stays on that host, which also keeps `X-API-Key`
//! (a header `reqwest` does not strip on its own) from following one away.

use super::when::Window;
use super::{Failure, Hit, Kind, Request, Searcher, mcp, one_line};
use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use std::time::Duration;

const NAME: &str = "searxng";

/// What SearXNG says of a page is a sentence or two; this is what is kept.
const SNIPPET_MAX_CHARS: usize = 500;

/// The longest URL the desk keeps for an instance.
const MAX_URL_CHARS: usize = 2048;

/// How many redirects on the instance's own host are followed.
const MAX_REDIRECTS: usize = 3;

/// The URL an instance may have, in the one shape it is kept: http or https,
/// a host, and a path if it is served under one; no user or password (a key
/// goes in the key field, to the keychain), no query (the search writes its
/// own) and no fragment. Any host: `localhost` and an address on this network
/// are the usual homes of a SearXNG, and the person, not a teammate, chooses
/// it. Blank is `None`.
pub fn instance_url(raw: &str) -> Result<Option<String>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let wrong = || {
        "Enter the SearXNG instance's full http or https address, such as http://localhost:8888."
            .to_string()
    };
    if raw.chars().count() > MAX_URL_CHARS || raw.chars().any(char::is_control) {
        return Err(wrong());
    }
    let url = url::Url::parse(raw).map_err(|_| wrong())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(wrong());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "The SearXNG address cannot carry a user or password. Put a key in its key field."
                .to_string(),
        );
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(
            "The SearXNG address is the instance itself, without a query or a fragment."
                .to_string(),
        );
    }
    let text = url.as_str().trim_end_matches('/');
    Ok(Some(text.to_string()))
}

/// The client SearXNG is asked with: the chain's own bounds, and redirects
/// held to the instance's host.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(super::TOTAL_BUDGET)
        .user_agent(concat!("hotline/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let named = attempt.previous().first().and_then(|url| url.host_str());
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.stop()
            } else if named.is_some_and(|host| {
                attempt
                    .url()
                    .host_str()
                    .is_some_and(|next| next.eq_ignore_ascii_case(host))
            }) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .unwrap_or_default()
}

pub struct Searxng {
    client: reqwest::Client,
    /// The instance, as [`instance_url`] keeps it.
    base: String,
    key: Option<String>,
}

impl Searxng {
    pub fn at(client: reqwest::Client, base: impl Into<String>, key: Option<String>) -> Self {
        Self {
            client,
            base: base.into(),
            key,
        }
    }

    /// `<instance>/search` with the query and the parameters SearXNG takes.
    fn url(&self, request: &Request) -> Option<url::Url> {
        let mut url = url::Url::parse(&self.base).ok()?;
        let path = format!("{}/search", url.path().trim_end_matches('/'));
        url.set_path(&path);
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("q", &request.query);
            query.append_pair("format", "json");
            if let Some(language) = request.language {
                query.append_pair("language", language.code);
            }
            if let Some(range) = request.window.and_then(|w| time_range(w, request.today)) {
                query.append_pair("time_range", range);
            }
        }
        Some(url)
    }
}

/// SearXNG's `time_range` for a window that runs up to now: the shortest of
/// day, week, month and year that holds it. A window wholly in the past has
/// no range SearXNG can say, so it is not sent.
pub(super) fn time_range(window: Window, today: NaiveDate) -> Option<&'static str> {
    if window.to < today - chrono::Duration::days(1) {
        return None;
    }
    let days = (today - window.from).num_days();
    Some(match days {
        ..=1 => "day",
        2..=7 => "week",
        8..=31 => "month",
        _ => "year",
    })
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
    #[serde(default)]
    content: String,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<String>,
}

/// An answer that is not a success, in words a person can act on. SearXNG
/// answers 403 when the instance does not serve the JSON format; a gateway in
/// front of it answers 401, or 403 too, when the key is missing or wrong.
fn status_failure(status: u16, body: &str, keyed: bool) -> Failure {
    let message = match status {
        403 if keyed => "searxng: the instance refused with 403: either its JSON format is off (add json under search.formats in its settings.yml) or it refused the key".to_string(),
        403 => "searxng: the instance refused with 403: its JSON format is off (add json under search.formats in its settings.yml)".to_string(),
        401 if keyed => "searxng: the instance refused the key (check it in Settings, Tools)".to_string(),
        401 => "searxng: the instance asks for a key (add one in Settings, Tools)".to_string(),
        _ => return Failure::status(NAME, status, body, keyed),
    };
    Failure::of(Kind::Status(status), message)
}

#[async_trait]
impl Searcher for Searxng {
    async fn search(&self, request: &Request) -> Result<Vec<Hit>, Failure> {
        let limit = request.depth;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let url = self.url(request).ok_or_else(|| {
            Failure::new(
                "searxng: the instance URL is not usable (set it again in Settings, Tools)",
            )
        })?;
        let key = self
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        let mut call = self.client.get(url).header("Accept", "application/json");
        if let Some(key) = key {
            call = call.bearer_auth(key).header("X-API-Key", key);
        }
        let response = call.send().await.map_err(|error| {
            if error.is_connect() {
                Failure::new(
                    "searxng: could not reach the instance (is it running at the URL in Settings, Tools?)",
                )
            } else {
                Failure::transport(NAME, &error)
            }
        })?;
        let status = response.status();
        let wait = mcp::retry_after(&response);
        let raw = mcp::read_capped(response, NAME).await?;
        if status.is_redirection() {
            return Err(Failure::of(
                Kind::Status(status.as_u16()),
                "searxng: the instance redirected away from its host, or too many times; set the URL it ends up at in Settings, Tools",
            ));
        }
        if !status.is_success() {
            return Err(status_failure(status.as_u16(), &raw, key.is_some()).after(wait));
        }
        let parsed: Response = serde_json::from_str(&raw).map_err(|_| {
            Failure::new(
                "searxng: the instance did not answer JSON (is the URL the instance itself?)",
            )
        })?;
        Ok(parsed
            .results
            .into_iter()
            .filter(|raw| !raw.url.trim().is_empty())
            .take(limit)
            .map(|raw| Hit {
                title: one_line(&raw.title),
                url: raw.url.trim().to_string(),
                snippet: one_line(&raw.content)
                    .chars()
                    .take(SNIPPET_MAX_CHARS)
                    .collect(),
                published: raw.published_date.filter(|at| !at.trim().is_empty()),
            })
            .collect())
    }
}
