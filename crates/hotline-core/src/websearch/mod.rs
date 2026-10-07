//! Web search for every teammate, keyless by default.
//!
//! Four providers answer, each a port of ketch's: Parallel, Exa, Keenable and
//! You.com. A search tries them in a fixed order and returns the first that
//! answers, so a rate-limited or broken one falls through to the next instead
//! of failing the tool. An optional key per provider lifts its limits and
//! moves it ahead of the keyless ones. Which providers a teammate's chain
//! holds is the desk's switches intersected with the teammate's own policy;
//! see [`effective_chain`].
//!
//! The room asks for one search and gets text. Nothing here holds a secret
//! longer than a call: keys come in with the chain and are scrubbed from every
//! error before it leaves.

mod exa;
mod keenable;
mod mcp;
mod parallel;
mod youcom;

use crate::contract::{PolicyMode, WebSearchPolicy, WebSearchProvider};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub use exa::Exa;
pub use keenable::Keenable;
pub use parallel::Parallel;
pub use youcom::Youcom;

/// How long one provider may take. Ketch's multi-backend timeout: it clears
/// a provider's slow path and still bounds the call.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the whole chain may take. Attempts are sequential, so without it
/// four slow providers would hold a search for forty seconds.
pub const TOTAL_BUDGET: Duration = Duration::from_secs(30);

pub const DEFAULT_LIMIT: usize = 8;
pub const MAX_LIMIT: usize = 20;
pub const MAX_QUERY_CHARS: usize = 400;

/// The most of one snippet a model is shown.
const SNIPPET_CHARS: usize = 300;

/// The providers in the order they are tried when no key promotes one: ketch's
/// rank order.
pub const ORDER: [WebSearchProvider; 4] = [
    WebSearchProvider::Parallel,
    WebSearchProvider::Exa,
    WebSearchProvider::Keenable,
    WebSearchProvider::Youcom,
];

/// The name a person reads for a provider.
pub fn display_name(provider: WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Parallel => "Parallel",
        WebSearchProvider::Exa => "Exa",
        WebSearchProvider::Keenable => "Keenable",
        WebSearchProvider::Youcom => "You.com",
    }
}

/// The lowercase id errors and the wire use.
pub fn id(provider: WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Parallel => "parallel",
        WebSearchProvider::Exa => "exa",
        WebSearchProvider::Keenable => "keenable",
        WebSearchProvider::Youcom => "youcom",
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
    /// The provider said the key it was given is no good, in a body it sent
    /// with a success status.
    pub(crate) rejected_key: bool,
}

impl Failure {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            rejected_key: false,
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

/// One provider, asked one question.
#[async_trait]
pub trait Searcher: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, Failure>;
}

/// A built searcher for a provider, and the key it holds, so the chain can
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
    pub youcom: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            parallel: parallel::ENDPOINT.into(),
            exa: exa::ENDPOINT.into(),
            keenable: keenable::BASE.into(),
            youcom: youcom::ENDPOINT.into(),
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
        WebSearchProvider::Youcom => Box::new(Youcom::at(client, &endpoints.youcom, key.clone())),
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
        "The webSearch setting must be an object whose disabled list names providers: parallel, exa, keenable or youcom."
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

/// The providers a teammate's search tries, in order.
///
/// The desk's switches decide what exists: a provider switched off there is
/// off for everyone. The teammate's policy then narrows it — absent or `all`
/// inherits whatever the desk has on, `none` leaves nothing, `some` keeps only
/// what it names. A provider with a key moves ahead of the keyless ones, each
/// group in [`ORDER`], as ketch's `autoPromoted` does.
pub fn effective_chain(
    disabled_on_desk: &HashSet<WebSearchProvider>,
    policy: Option<&WebSearchPolicy>,
    keyed: &HashSet<WebSearchProvider>,
) -> Vec<WebSearchProvider> {
    let mut chain: Vec<WebSearchProvider> = ORDER
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
        .collect();
    chain.sort_by_key(|provider| !keyed.contains(provider));
    chain
}

/// A search that tried the chain: who answered, what they said, and who
/// failed first.
#[derive(Debug)]
pub struct Answered {
    pub provider: WebSearchProvider,
    pub hits: Vec<Hit>,
    pub failed: Vec<(WebSearchProvider, String)>,
}

/// The fallback chain: each attempt in order, each bounded by its own timeout,
/// the whole bounded by a budget.
pub struct Chain {
    attempts: Vec<Attempt>,
    timeout: Duration,
    budget: Duration,
}

impl Chain {
    pub fn new(attempts: Vec<Attempt>) -> Self {
        Self {
            attempts,
            timeout: ATTEMPT_TIMEOUT,
            budget: TOTAL_BUDGET,
        }
    }

    /// Tighter bounds, so a test of the fallback does not take ten seconds.
    pub fn with_bounds(mut self, timeout: Duration, budget: Duration) -> Self {
        self.timeout = timeout;
        self.budget = budget;
        self
    }

    /// The first success. A response with no results is a success: falling
    /// through on an empty set would turn every query nothing matches into a
    /// sweep of every provider. The error names each provider tried and why.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Answered, String> {
        let deadline = Instant::now() + self.budget;
        let mut failed: Vec<(WebSearchProvider, String)> = Vec::new();
        for attempt in &self.attempts {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.over_budget(&failed));
            }
            let allowed = remaining.min(self.timeout);
            let outcome =
                tokio::time::timeout(allowed, attempt.searcher.search(query, limit)).await;
            let reason = match outcome {
                Ok(Ok(hits)) => {
                    return Ok(Answered {
                        provider: attempt.provider,
                        hits,
                        failed,
                    });
                }
                Ok(Err(failure)) => scrub(&failure.message, attempt.key.as_deref()),
                Err(_) => format!("{}: timed out after {allowed:?}", id(attempt.provider)),
            };
            failed.push((attempt.provider, reason));
            if Instant::now() >= deadline {
                return Err(self.over_budget(&failed));
            }
        }
        Err(format!(
            "all {} providers failed ({})",
            self.attempts.len(),
            reasons(&failed)
        ))
    }

    fn over_budget(&self, failed: &[(WebSearchProvider, String)]) -> String {
        if failed.is_empty() {
            format!(
                "web search exceeded its {:?} budget before any provider answered",
                self.budget
            )
        } else {
            format!(
                "web search exceeded its {:?} budget after {} provider failures ({})",
                self.budget,
                failed.len(),
                reasons(failed)
            )
        }
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

/// What the model reads: each result's title, URL and a trimmed snippet, then
/// the provider that answered.
pub fn render(answered: &Answered) -> String {
    let mut out = String::new();
    if answered.hits.is_empty() {
        out.push_str("No results.\n");
    }
    for (index, hit) in answered.hits.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", index + 1, hit.title, hit.url));
        let snippet = bounded(&one_line(&hit.snippet), SNIPPET_CHARS);
        if !snippet.is_empty() {
            out.push_str(&format!("   {snippet}\n"));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "Searched with {}.",
        display_name(answered.provider)
    ));
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
