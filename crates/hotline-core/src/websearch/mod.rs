//! Web search for every teammate, keyless by default.
//!
//! Four providers answer, each a port of ketch's: Parallel, Exa, Keenable and
//! Firecrawl. A search asks every one that is switched on at once, as ketch's
//! `multi.go` does, and fuses what comes back by Reciprocal Rank Fusion; a
//! slow or broken provider costs its list, not the search. An optional key per
//! provider gives its list a little more weight. Which providers a teammate
//! searches with is the desk's switches intersected with the teammate's own
//! policy; see [`effective_chain`].
//!
//! The results are then made fit to read, by `clean`, `rank` and `query`:
//! error pages and page chrome out, duplicates and language mirrors merged,
//! and a few small rules about what the query asked for. The room asks for one
//! search and gets text. Nothing here holds a secret longer than a call: keys
//! come in with the providers and are scrubbed from every error.

pub mod canonical;
pub mod clean;
mod exa;
mod firecrawl;
mod keenable;
mod mcp;
mod parallel;
pub mod query;
pub mod rank;

use crate::contract::{PolicyMode, WebSearchPolicy, WebSearchProvider};
use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use query::{Language, Query};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

pub use exa::Exa;
pub use firecrawl::Firecrawl;
pub use keenable::Keenable;
pub use parallel::Parallel;

/// How long one provider may take. Fan-out is parallel, so the search takes
/// about the slowest provider, and this is what bounds it.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the whole search may take. Whatever arrived by then is used.
pub const TOTAL_BUDGET: Duration = Duration::from_secs(6);

pub const DEFAULT_LIMIT: usize = 8;
pub const MAX_LIMIT: usize = 20;
pub const MAX_QUERY_CHARS: usize = 400;

/// The most of one snippet a model is shown.
const SNIPPET_CHARS: usize = 300;

/// How many results each provider is asked for: more than the limit, so fusion
/// has overlap to work with, and no more than a provider's useful depth.
pub fn depth_for(limit: usize) -> usize {
    (limit * 2).clamp(10, 20)
}

/// The providers in ketch's AutoRank order: Parallel 70, Exa 80, Keenable 90,
/// Firecrawl 110. Fusion does not depend on it beyond breaking ties.
pub const ORDER: [WebSearchProvider; 4] = [
    WebSearchProvider::Parallel,
    WebSearchProvider::Exa,
    WebSearchProvider::Keenable,
    WebSearchProvider::Firecrawl,
];

/// The name a person reads for a provider.
pub fn display_name(provider: WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Parallel => "Parallel",
        WebSearchProvider::Exa => "Exa",
        WebSearchProvider::Keenable => "Keenable",
        WebSearchProvider::Firecrawl => "Firecrawl",
    }
}

/// The lowercase id errors and the wire use.
pub fn id(provider: WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Parallel => "parallel",
        WebSearchProvider::Exa => "exa",
        WebSearchProvider::Keenable => "keenable",
        WebSearchProvider::Firecrawl => "firecrawl",
    }
}

/// One search result, in the provider-neutral shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Why one provider did not answer. The text never holds a key: transport
/// errors are reduced to a reason, not the URL they failed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
}

impl Failure {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// A transport error, reduced to a reason: `reqwest` puts the request URL
    /// in its errors, and Exa's key is in its URL.
    pub(crate) fn transport(name: &str, error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::new(format!("{name}: request failed: timed out"))
        } else {
            Self::new(format!("{name}: request failed: transport error"))
        }
    }

    /// An HTTP status that was not a success, in words a person can act on.
    pub(crate) fn status(name: &str, status: u16, body: &str, keyed: bool) -> Self {
        Self::new(match status {
            401 | 403 if keyed => {
                format!("{name}: invalid API key (check it in Settings, Tools)")
            }
            429 if keyed => format!("{name}: rate limited"),
            429 => format!("{name}: rate limited (add a key in Settings, Tools, to lift the cap)"),
            402 => format!("{name}: credits exhausted (see your {name} plan)"),
            _ => {
                let detail = Self::detail(body);
                if detail.is_empty() {
                    format!("{name} returned status {status}")
                } else {
                    format!("{name} returned status {status}: {detail}")
                }
            }
        })
    }

    /// A server's own words, on one line and bounded.
    pub(crate) fn detail(text: &str) -> String {
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        bounded(&one_line, 200)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `text` cut to at most `max` characters, ending in an ellipsis when it was cut.
pub(crate) fn bounded(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Everything on one line, runs of whitespace collapsed to one space.
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a provider is asked.
#[derive(Clone, Debug)]
pub struct Request {
    pub query: String,
    /// How many results to ask for.
    pub depth: usize,
    /// The query's language, when it is detected with confidence and is not
    /// English. Only passed where a provider's API takes it.
    pub language: Option<Language>,
    /// The query asks for the news or for now.
    pub recency: bool,
}

impl Request {
    pub fn new(query: &Query, limit: usize) -> Self {
        Self {
            query: query.text.clone(),
            depth: depth_for(limit),
            language: query.non_english(),
            recency: query.recency,
        }
    }

    /// A plain query at a depth, for a test or a smoke run.
    pub fn plain(query: &str, depth: usize) -> Self {
        Self {
            query: query.to_string(),
            depth,
            language: None,
            recency: false,
        }
    }

    /// What to add to a natural-language `objective` for the providers whose
    /// API takes the goal as a sentence and has no language or date field.
    pub(crate) fn objective(&self) -> String {
        let mut text = self.query.clone();
        if let Some(language) = self.language {
            text.push_str(&format!(" Prefer pages written in {}.", language.name));
        }
        if self.recency {
            text.push_str(" Prefer recent news from the past week over index and tag pages.");
        }
        text
    }
}

/// One provider, asked one question.
#[async_trait]
pub trait Searcher: Send + Sync {
    async fn search(&self, request: &Request) -> Result<Vec<Hit>, Failure>;
}

/// A built searcher for a provider, and the key it holds, so the search can
/// scrub it from whatever goes wrong.
pub struct Attempt {
    pub provider: WebSearchProvider,
    pub searcher: Box<dyn Searcher>,
    pub key: Option<String>,
}

/// Where each provider is reached. The real ones by default; a test points
/// them at a mock server on localhost.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub parallel: String,
    pub exa: String,
    pub keenable: String,
    pub firecrawl: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            parallel: parallel::ENDPOINT.into(),
            exa: exa::ENDPOINT.into(),
            keenable: keenable::BASE.into(),
            firecrawl: firecrawl::BASE.into(),
        }
    }
}

/// The real provider, built over `client`, with its key if it has one.
pub fn attempt(
    provider: WebSearchProvider,
    client: &reqwest::Client,
    key: Option<String>,
    endpoints: &Endpoints,
) -> Attempt {
    let client = client.clone();
    let searcher: Box<dyn Searcher> = match provider {
        WebSearchProvider::Parallel => {
            Box::new(Parallel::at(client, &endpoints.parallel, key.clone()))
        }
        WebSearchProvider::Exa => Box::new(Exa::at(client, &endpoints.exa, key.clone())),
        WebSearchProvider::Keenable => {
            Box::new(Keenable::at(client, &endpoints.keenable, key.clone()))
        }
        WebSearchProvider::Firecrawl => {
            Box::new(Firecrawl::at(client, &endpoints.firecrawl, key.clone()))
        }
    };
    Attempt {
        provider,
        searcher,
        key,
    }
}

/// The providers the desk has switched off: `settings.webSearch.disabled`.
/// Absent is all on, and a name this build does not know is ignored.
pub fn disabled_on_desk(
    settings: &serde_json::Map<String, serde_json::Value>,
) -> HashSet<WebSearchProvider> {
    settings
        .get("webSearch")
        .and_then(|value| value.get("disabled"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|name| serde_json::from_value(name.clone()).ok())
        .collect()
}

/// Checks a `webSearch` setting and writes it back in its one shape.
pub fn normalize_setting(value: &serde_json::Value) -> Result<serde_json::Value, String> {
    let wrong = || {
        "The webSearch setting must be an object whose disabled list names providers: parallel, exa, keenable or firecrawl."
            .to_string()
    };
    let object = value.as_object().ok_or_else(wrong)?;
    if object.keys().any(|key| key != "disabled") {
        return Err(wrong());
    }
    let listed = match object.get("disabled") {
        None => Vec::new(),
        Some(list) => list.as_array().ok_or_else(wrong)?.clone(),
    };
    let mut disabled = HashSet::new();
    for name in listed {
        disabled.insert(serde_json::from_value::<WebSearchProvider>(name).map_err(|_| wrong())?);
    }
    let disabled: Vec<&str> = ORDER
        .into_iter()
        .filter(|provider| disabled.contains(provider))
        .map(id)
        .collect();
    Ok(serde_json::json!({ "disabled": disabled }))
}

/// The providers a teammate's search asks, in [`ORDER`].
///
/// The desk's switches decide what exists: a provider switched off there is
/// off for everyone. The teammate's policy then narrows it: absent or `all`
/// inherits whatever the desk has on, `none` leaves nothing, `some` keeps only
/// what it names.
pub fn effective_chain(
    disabled_on_desk: &HashSet<WebSearchProvider>,
    policy: Option<&WebSearchPolicy>,
) -> Vec<WebSearchProvider> {
    ORDER
        .into_iter()
        .filter(|provider| !disabled_on_desk.contains(provider))
        .filter(|provider| match policy {
            None => true,
            Some(policy) => match policy.mode {
                PolicyMode::All => true,
                PolicyMode::None => false,
                PolicyMode::Some => policy.providers.contains(provider),
            },
        })
        .collect()
}

/// A search that was answered: the results, who answered, and who did not.
#[derive(Debug)]
pub struct Answered {
    pub hits: Vec<Hit>,
    /// Said when the query's rare words matched no result.
    pub note: Option<String>,
    pub answered: Vec<WebSearchProvider>,
    pub failed: Vec<(WebSearchProvider, String)>,
}

/// The fan-out: every provider at once, each bounded by its own timeout, the
/// whole by a budget, and what arrived fused.
pub struct Search {
    attempts: Vec<Attempt>,
    timeout: Duration,
    budget: Duration,
}

enum Outcome {
    Hits(Vec<Hit>),
    Failed(String),
}

impl Search {
    pub fn new(attempts: Vec<Attempt>) -> Self {
        Self {
            attempts,
            timeout: ATTEMPT_TIMEOUT,
            budget: TOTAL_BUDGET,
        }
    }

    /// Tighter bounds, so a test of the timeouts does not take seconds.
    pub fn with_bounds(mut self, timeout: Duration, budget: Duration) -> Self {
        self.timeout = timeout;
        self.budget = budget;
        self
    }

    /// Asks every provider and fuses the answers. A response with no results
    /// is an answer. The error, when none answered, names each provider and why.
    pub async fn run(&self, query: &Query, limit: usize) -> Result<Answered, String> {
        let request = Request::new(query, limit);
        let mut pending: FuturesUnordered<_> = self
            .attempts
            .iter()
            .enumerate()
            .map(|(index, attempt)| {
                let request = &request;
                async move {
                    let outcome =
                        tokio::time::timeout(self.timeout, attempt.searcher.search(request)).await;
                    let outcome = match outcome {
                        Ok(Ok(hits)) => Outcome::Hits(hits),
                        Ok(Err(failure)) => {
                            Outcome::Failed(scrub(&failure.message, attempt.key.as_deref()))
                        }
                        Err(_) => Outcome::Failed(format!(
                            "{}: timed out after {:?}",
                            id(attempt.provider),
                            self.timeout
                        )),
                    };
                    (index, outcome)
                }
            })
            .collect();
        let mut outcomes: Vec<Option<Outcome>> = self.attempts.iter().map(|_| None).collect();
        let deadline = tokio::time::sleep(self.budget);
        tokio::pin!(deadline);
        let mut over_budget = false;
        loop {
            tokio::select! {
                next = pending.next() => match next {
                    Some((index, outcome)) => outcomes[index] = Some(outcome),
                    None => break,
                },
                () = &mut deadline => {
                    over_budget = true;
                    break;
                }
            }
        }
        drop(pending);

        let mut sources = Vec::new();
        let mut answered = Vec::new();
        let mut failed = Vec::new();
        for (attempt, outcome) in self.attempts.iter().zip(outcomes) {
            match outcome {
                Some(Outcome::Hits(hits)) => {
                    answered.push(attempt.provider);
                    sources.push(rank::Source {
                        provider: attempt.provider,
                        keyed: attempt.key.is_some(),
                        hits,
                    });
                }
                Some(Outcome::Failed(reason)) => failed.push((attempt.provider, reason)),
                None => failed.push((
                    attempt.provider,
                    format!(
                        "{}: timed out after {:?}",
                        id(attempt.provider),
                        self.budget
                    ),
                )),
            }
        }
        if answered.is_empty() {
            return Err(if over_budget {
                format!(
                    "web search exceeded its {:?} budget before any provider answered ({})",
                    self.budget,
                    reasons(&failed)
                )
            } else {
                format!(
                    "all {} providers failed ({})",
                    self.attempts.len(),
                    reasons(&failed)
                )
            });
        }
        let ranked = rank::rank(query, sources, limit);
        Ok(Answered {
            hits: ranked.hits,
            note: ranked.note,
            answered,
            failed,
        })
    }
}

fn reasons(failed: &[(WebSearchProvider, String)]) -> String {
    failed
        .iter()
        .map(|(provider, reason)| {
            // A reason already begins with its provider's id, as ketch's do.
            if reason.starts_with(id(*provider)) {
                reason.clone()
            } else {
                format!("{}: {reason}", id(*provider))
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// `text` with every occurrence of the key replaced. Belt and braces: no
/// provider puts its key in an error, and this is why one that did would not
/// reach the model.
fn scrub(text: &str, key: Option<&str>) -> String {
    match key {
        Some(key) if !key.is_empty() => text.replace(key, "[key]"),
        _ => text.to_string(),
    }
}

/// A provider's failure in a word, for the line that says who was left out.
fn why(reason: &str) -> &'static str {
    if reason.contains("timed out") {
        "timed out"
    } else if reason.contains("rate limited") {
        "rate limited"
    } else {
        "failed"
    }
}

/// What the model reads: a note when the query's rare words matched nothing,
/// each result's title, URL and a trimmed snippet, then who answered.
pub fn render(answered: &Answered) -> String {
    let mut out = String::new();
    if let Some(note) = &answered.note {
        out.push_str(note);
        out.push_str("\n\n");
    }
    if answered.hits.is_empty() {
        out.push_str("No results.\n\n");
    }
    for (index, hit) in answered.hits.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", index + 1, hit.title, hit.url));
        let snippet = bounded(&one_line(&hit.snippet), SNIPPET_CHARS);
        if !snippet.is_empty() {
            out.push_str(&format!("   {snippet}\n"));
        }
        out.push('\n');
    }
    let names: Vec<&str> = answered.answered.iter().map(|p| display_name(*p)).collect();
    out.push_str(&format!("Searched with {}", names.join(", ")));
    if !answered.failed.is_empty() {
        let left: Vec<String> = answered
            .failed
            .iter()
            .map(|(provider, reason)| format!("{} {}", display_name(*provider), why(reason)))
            .collect();
        out.push_str(&format!(" ({})", left.join(", ")));
    }
    out.push('.');
    out
}

/// A shared client for the providers: no proxy surprises, a connect bound, and
/// the attempt timeout as a backstop to the chain's own.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(TOTAL_BUDGET)
        .user_agent(concat!("hotline/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// The keys a chain was given, by provider: the vault's, read at call time.
pub type Keys = HashMap<WebSearchProvider, String>;

#[cfg(test)]
pub(crate) mod tests;
