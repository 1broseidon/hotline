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
pub mod lexical;
mod mcp;
mod parallel;
pub mod query;
pub mod rank;
pub mod when;

use crate::contract::{PolicyMode, WebSearchPolicy, WebSearchProvider};
use async_trait::async_trait;
use chrono::NaiveDate;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use query::{Language, Query};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use when::Window;

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
    /// When the page was published, as the provider said it: a date, a time,
    /// or "2 hours ago". Not every provider says.
    pub published: Option<String>,
}

/// Why one provider did not answer. The text never holds a key: transport
/// errors are reduced to a reason, not the URL they failed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// What went wrong, for the cooldown and the footer. The message is for a
    /// person and may be reworded; this is what the code decides by.
    pub kind: Kind,
    pub message: String,
    /// How long the provider asked to be left alone, when it said.
    pub retry_after: Option<Duration>,
}

/// The kinds of failure a search treats differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// HTTP 429.
    RateLimited,
    /// HTTP 402: the plan's credits are spent.
    OutOfCredits,
    /// The request, or the search's wait for it, ran out of time.
    TimedOut,
    /// Any other HTTP status that was not a success.
    Status(u16),
    /// Not asked, because an earlier failure left it resting.
    Resting,
    /// A transport error, an answer that would not decode, or a refusal in
    /// the body of a success.
    Other,
}

impl Kind {
    /// The failure in a word, for the line that says who was left out.
    fn word(self) -> &'static str {
        match self {
            Kind::Resting => "cooling down",
            Kind::TimedOut => "timed out",
            Kind::RateLimited => "rate limited",
            Kind::OutOfCredits | Kind::Status(_) | Kind::Other => "failed",
        }
    }
}

impl Failure {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self::of(Kind::Other, message)
    }

    pub(crate) fn of(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: None,
        }
    }

    /// The wait the provider asked for, from its `Retry-After` header.
    pub(crate) fn after(mut self, wait: Option<Duration>) -> Self {
        self.retry_after = self.retry_after.or(wait);
        self
    }

    /// A transport error, reduced to a reason: `reqwest` puts the request URL
    /// in its errors, and Exa's key is in its URL.
    pub(crate) fn transport(name: &str, error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::of(Kind::TimedOut, format!("{name}: request failed: timed out"))
        } else {
            Self::new(format!("{name}: request failed: transport error"))
        }
    }

    /// An HTTP status that was not a success, in words a person can act on.
    pub(crate) fn status(name: &str, status: u16, body: &str, keyed: bool) -> Self {
        // Firecrawl says how long in its body: `retry_after_seconds`.
        let wait = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("retry_after_seconds")?.as_u64())
            .map(Duration::from_secs);
        let kind = match status {
            429 => Kind::RateLimited,
            402 => Kind::OutOfCredits,
            _ => Kind::Status(status),
        };
        let message = match status {
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
        };
        Self::of(kind, message).after(wait)
    }

    /// How long to leave the provider alone after this failure, if at all: a
    /// rate limit for what it asked or a minute, a timeout or a server error
    /// for a short while. Other failures (a bad key, a changed API) are not
    /// helped by waiting.
    pub(crate) fn cooldown(&self) -> Option<Duration> {
        match self.kind {
            Kind::RateLimited | Kind::OutOfCredits => Some(
                self.retry_after
                    .unwrap_or(RATE_LIMIT_COOLDOWN)
                    .min(LONGEST_COOLDOWN),
            ),
            Kind::TimedOut | Kind::Status(500..=599) => Some(SLOW_COOLDOWN),
            Kind::Status(_) | Kind::Resting | Kind::Other => None,
        }
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

/// Waits after a rate limit, and after a timeout or a server error.
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
const SLOW_COOLDOWN: Duration = Duration::from_secs(20);
/// No provider is left alone longer than this, whatever it asks.
const LONGEST_COOLDOWN: Duration = Duration::from_secs(15 * 60);

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
    /// The days the query is about, when it is about news or a date.
    pub window: Option<Window>,
    /// The day the query was made.
    pub today: NaiveDate,
}

impl Request {
    pub fn new(query: &Query, limit: usize) -> Self {
        Self {
            query: query.text.clone(),
            depth: depth_for(limit),
            language: query.non_english(),
            window: query.window,
            today: query.today,
        }
    }

    /// What to add to a natural-language `objective` for the providers whose
    /// API takes the goal as a sentence and has no language or date field.
    pub(crate) fn objective(&self) -> String {
        let mut text = self.query.clone();
        if let Some(language) = self.language {
            text.push_str(&format!(" Prefer pages written in {}.", language.name));
        }
        if let Some(window) = self.window {
            // Neither API has a date field, so the sentence is the only way.
            text.push_str(&format!(
                " Today is {}. Only pages published on or after {}, and stories rather than index, tag or section pages.",
                self.today, window.from
            ));
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
#[derive(Clone, Debug)]
pub struct Answered {
    pub hits: Vec<Hit>,
    /// Said when the query's rare words matched no result.
    pub note: Option<String>,
    pub answered: Vec<WebSearchProvider>,
    pub failed: Vec<(WebSearchProvider, Failure)>,
}

/// How long a fused answer is kept for the same question.
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
/// How many answers are kept.
const CACHE_ENTRIES: usize = 100;

/// What a room remembers between searches: which providers are resting, and
/// the answers it gave lately, so a repeated question is steady and instant.
pub struct State {
    cooling: std::sync::Mutex<HashMap<WebSearchProvider, (Instant, &'static str)>>,
    cache: std::sync::Mutex<HashMap<String, (Instant, Answered)>>,
    ttl: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self::with_ttl(CACHE_TTL)
    }
}

impl State {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            cooling: Default::default(),
            cache: Default::default(),
            ttl,
        }
    }

    /// Why a provider is resting, if it is.
    fn resting(&self, provider: WebSearchProvider) -> Option<&'static str> {
        let mut cooling = self
            .cooling
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match cooling.get(&provider) {
            Some((until, why)) if *until > Instant::now() => Some(*why),
            Some(_) => {
                cooling.remove(&provider);
                None
            }
            None => None,
        }
    }

    fn rest(&self, provider: WebSearchProvider, wait: Duration, why: &'static str) {
        self.cooling
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(provider, (Instant::now() + wait, why));
    }

    fn recall(&self, key: &str) -> Option<Answered> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match cache.get(key) {
            Some((at, answered)) if at.elapsed() < self.ttl => Some(answered.clone()),
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }

    fn remember(&self, key: String, answered: &Answered) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ttl = self.ttl;
        cache.retain(|_, (at, _)| at.elapsed() < ttl);
        while cache.len() >= CACHE_ENTRIES {
            let oldest = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(oldest) => cache.remove(&oldest),
                None => break,
            };
        }
        cache.insert(key, (Instant::now(), answered.clone()));
    }
}

/// The fan-out: every provider at once, each bounded by its own timeout, the
/// whole by a budget, and what arrived fused.
pub struct Search {
    attempts: Vec<Attempt>,
    timeout: Duration,
    budget: Duration,
    state: Arc<State>,
}

enum Outcome {
    Hits(Vec<Hit>),
    Failed(Failure),
}

impl Search {
    pub fn new(attempts: Vec<Attempt>) -> Self {
        Self {
            attempts,
            timeout: ATTEMPT_TIMEOUT,
            budget: TOTAL_BUDGET,
            state: Arc::default(),
        }
    }

    /// Shares a room's memory of resting providers and recent answers.
    pub fn with_state(mut self, state: Arc<State>) -> Self {
        self.state = state;
        self
    }

    /// What makes two searches the same question.
    fn cache_key(&self, query: &Query, limit: usize) -> String {
        let words = query.text.to_lowercase();
        let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
        let providers: Vec<&str> = self.attempts.iter().map(|a| id(a.provider)).collect();
        format!("{words}\u{1f}{limit}\u{1f}{}", providers.join(","))
    }

    /// Asks every provider and fuses the answers. A response with no results
    /// is an answer. The error, when none answered, names each provider and why.
    pub async fn run(&self, query: &Query, limit: usize) -> Result<Answered, String> {
        let key = self.cache_key(query, limit);
        if let Some(answered) = self.state.recall(&key) {
            return Ok(answered);
        }
        let request = Request::new(query, limit);
        let mut outcomes: Vec<Option<Outcome>> = self.attempts.iter().map(|_| None).collect();
        // A provider that was rate limited or fell over is left alone a while.
        for (index, attempt) in self.attempts.iter().enumerate() {
            if let Some(why) = self.state.resting(attempt.provider) {
                outcomes[index] = Some(Outcome::Failed(Failure::of(
                    Kind::Resting,
                    format!("{}: cooling down ({why})", id(attempt.provider)),
                )));
            }
        }
        let mut pending: FuturesUnordered<_> = self
            .attempts
            .iter()
            .enumerate()
            .filter(|(index, _)| outcomes[*index].is_none())
            .map(|(index, attempt)| {
                let request = &request;
                async move {
                    let outcome =
                        tokio::time::timeout(self.timeout, attempt.searcher.search(request)).await;
                    let outcome = match outcome {
                        Ok(Ok(hits)) => Outcome::Hits(hits),
                        Ok(Err(failure)) => Outcome::Failed(Failure {
                            message: scrub(&failure.message, attempt.key.as_deref()),
                            ..failure
                        }),
                        Err(_) => Outcome::Failed(Failure::of(
                            Kind::TimedOut,
                            format!(
                                "{}: timed out after {:?}",
                                id(attempt.provider),
                                self.timeout
                            ),
                        )),
                    };
                    (index, outcome)
                }
            })
            .collect();
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
            let failure = match outcome {
                Some(Outcome::Hits(hits)) => {
                    answered.push(attempt.provider);
                    sources.push(rank::Source {
                        keyed: attempt.key.is_some(),
                        hits,
                    });
                    continue;
                }
                Some(Outcome::Failed(failure)) => failure,
                None => Failure::of(
                    Kind::TimedOut,
                    format!(
                        "{}: timed out after {:?}",
                        id(attempt.provider),
                        self.budget
                    ),
                ),
            };
            if let Some(wait) = failure.cooldown() {
                self.state.rest(attempt.provider, wait, failure.kind.word());
            }
            failed.push((attempt.provider, failure));
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
        let result = Answered {
            hits: ranked.hits,
            note: ranked.note,
            answered,
            failed,
        };
        if !result.hits.is_empty() {
            self.state.remember(key, &result);
        }
        Ok(result)
    }
}

fn reasons(failed: &[(WebSearchProvider, Failure)]) -> String {
    failed
        .iter()
        .map(|(provider, failure)| {
            // A reason already begins with its provider's id, as ketch's do.
            if failure.message.starts_with(id(*provider)) {
                failure.message.clone()
            } else {
                format!("{}: {}", id(*provider), failure.message)
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
        if snippet.is_empty() {
            out.push_str("   (no preview)\n");
        } else {
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
            .map(|(provider, failure)| {
                format!("{} {}", display_name(*provider), failure.kind.word())
            })
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
