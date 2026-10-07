//! The providers against mock servers on localhost, and the chain over them.
//! No test here reaches the network.

use super::*;
use crate::contract::{PolicyMode, WebSearchPolicy, WebSearchProvider as P};
use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// What one mock request looked like.
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Seen {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// What a mock answers.
#[derive(Clone)]
pub(crate) struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    delay: Duration,
}

impl Reply {
    pub(crate) fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    /// One SSE frame carrying `rpc`, as the hosted MCP servers answer.
    pub(crate) fn sse(rpc: impl AsRef<str>) -> Self {
        Self {
            content_type: "text/event-stream",
            body: format!("event: message\ndata: {}\n\n", rpc.as_ref()),
            ..Self::json("")
        }
    }

    /// An MCP tool result whose one text block is `text`.
    pub(crate) fn tool_text(text: &str) -> String {
        json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":text}]}})
            .to_string()
    }

    pub(crate) fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            ..Self::json(body)
        }
    }

    pub(crate) fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

pub(crate) struct Mock {
    /// `http://127.0.0.1:port`
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Mock {
    /// `/mcp` on this server.
    pub(crate) fn mcp(&self) -> String {
        format!("{}/mcp", self.base)
    }

    fn only(&self) -> Seen {
        let mut seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "expected exactly one request");
        seen.remove(0)
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn mock(reply: Reply) -> Mock {
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let record = seen.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let record = record.clone();
        let reply = reply.clone();
        async move {
            record.lock().unwrap().push(Seen {
                path: uri.path_and_query().unwrap().to_string(),
                headers,
                body: bytes.to_vec(),
            });
            tokio::time::sleep(reply.delay).await;
            (
                StatusCode::from_u16(reply.status).unwrap(),
                [(header::CONTENT_TYPE, reply.content_type)],
                reply.body,
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Mock { base, seen, task }
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

const KEY: &str = "k-never-shown-0123456789";

// Parallel ------------------------------------------------------------------

#[tokio::test]
async fn parallel_sends_the_search_call_and_maps_its_results() {
    let long = "界".repeat(301);
    let payload = json!({"results":[
        {"url":"https://go.dev/blog/context","title":"Go\n Concurrency  Patterns","excerpts":[" First excerpt. ","Second excerpt."]},
        {"url":"https://pkg.go.dev/context","title":"context package","excerpts":[]},
        {"url":"","title":"missing URL","excerpts":["x"]},
        {"url":"https://example.com/long","title":"long","excerpts":[long]},
    ]})
    .to_string();
    let server = mock(Reply::json(Reply::tool_text(&payload))).await;
    let parallel = Parallel::at(http(), server.mcp(), None);
    let hits = parallel.search("go context cancellation", 5).await.unwrap();

    assert_eq!(
        hits[0],
        Hit {
            title: "Go Concurrency Patterns".into(),
            url: "https://go.dev/blog/context".into(),
            snippet: "First excerpt.".into()
        }
    );
    assert_eq!(hits[1].snippet, "");
    assert_eq!(hits.len(), 3, "the result with no URL is skipped");
    assert_eq!(hits[2].snippet.chars().count(), 300);
    assert!(hits[2].snippet.ends_with('…'));

    let seen = server.only();
    assert_eq!(
        seen.header("accept"),
        Some("application/json, text/event-stream")
    );
    assert!(seen.header("authorization").is_none());
    let body = seen.json();
    assert_eq!(body["method"], "tools/call");
    assert_eq!(body["params"]["name"], "web_search");
    assert_eq!(
        body["params"]["arguments"]["objective"],
        "go context cancellation"
    );
    assert_eq!(
        body["params"]["arguments"]["search_queries"],
        json!(["go context cancellation"])
    );
}

#[tokio::test]
async fn parallel_applies_the_limit_locally_and_sends_a_key_as_a_bearer() {
    let payload = json!({"results":[
        {"url":"https://e.com/1","title":"one","excerpts":["a"]},
        {"url":"https://e.com/2","title":"two","excerpts":["b"]},
    ]})
    .to_string();
    let server = mock(Reply::json(Reply::tool_text(&payload))).await;
    let parallel = Parallel::at(http(), server.mcp(), Some(KEY.into()));
    let hits = parallel.search("q", 1).await.unwrap();
    assert_eq!(hits.len(), 1);
    let seen = server.only();
    assert_eq!(
        seen.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(!seen.path.contains(KEY));
}

#[tokio::test]
async fn parallel_with_no_text_or_a_bad_status_fails() {
    let empty = mock(Reply::json(json!({"result":{"content":[]}}).to_string())).await;
    let error = Parallel::at(http(), empty.mcp(), None)
        .search("q", 5)
        .await
        .unwrap_err();
    assert_eq!(error.message, "parallel response contained no text results");

    let limited = mock(Reply::status(429, "slow down")).await;
    let error = Parallel::at(http(), limited.mcp(), None)
        .search("q", 5)
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("parallel: rate limited"),
        "{error}"
    );
}

// Exa -----------------------------------------------------------------------

const EXA_TEXT: &str = "Title: First\nURL: https://example.com/one\nPublished date: 2026-01-01\nHighlights:\nA summary.\nMore.\n---\nTitle: Second\nURL: https://example.com/two\nAnother summary.\n---\nTitle: No URL\nnothing";

#[tokio::test]
async fn exa_sends_the_call_keyless_and_parses_its_text_blocks_from_sse() {
    let server = mock(Reply::sse(Reply::tool_text(EXA_TEXT))).await;
    let hits = Exa::at(http(), server.mcp(), None)
        .search("rust", 5)
        .await
        .unwrap();
    assert_eq!(
        hits,
        vec![
            Hit {
                title: "First".into(),
                url: "https://example.com/one".into(),
                snippet: "A summary.".into()
            },
            Hit {
                title: "Second".into(),
                url: "https://example.com/two".into(),
                snippet: "Another summary.".into()
            },
        ]
    );
    let seen = server.only();
    assert_eq!(seen.path, "/mcp");
    assert!(seen.header("authorization").is_none());
    let body = seen.json();
    assert_eq!(body["params"]["name"], "web_search_exa");
    let arguments = &body["params"]["arguments"];
    assert_eq!(arguments["query"], "rust");
    assert_eq!(arguments["numResults"], 5);
    assert_eq!(arguments["type"], "auto");
    assert_eq!(arguments["livecrawl"], "fallback");
    assert_eq!(arguments["contextMaxCharacters"], 3000);
}

#[tokio::test]
async fn exa_passes_a_key_in_the_query_and_respects_the_limit() {
    let server = mock(Reply::sse(Reply::tool_text(EXA_TEXT))).await;
    let hits = Exa::at(http(), server.mcp(), Some(KEY.into()))
        .search("rust", 1)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(server.only().path, format!("/mcp?exaApiKey={KEY}"));
}

#[tokio::test]
async fn exa_with_nothing_matched_is_an_empty_success() {
    let server = mock(Reply::sse(json!({"result":{"content":[]}}).to_string())).await;
    let hits = Exa::at(http(), server.mcp(), None)
        .search("zzz", 5)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn exa_statuses_read_as_a_person_would_and_never_carry_the_key() {
    let denied = mock(Reply::status(401, "")).await;
    let error = Exa::at(http(), denied.mcp(), Some(KEY.into()))
        .search("q", 5)
        .await
        .unwrap_err();
    assert!(error.message.contains("invalid API key"), "{error}");
    assert!(!error.message.contains(KEY));

    let broken = mock(Reply::status(500, "boom")).await;
    let error = Exa::at(http(), broken.mcp(), None)
        .search("q", 5)
        .await
        .unwrap_err();
    assert_eq!(error.message, "exa returned status 500: boom");
}

#[tokio::test]
async fn a_transport_failure_does_not_put_the_keyed_url_in_the_error() {
    // Nothing listens here, so the connection is refused and reqwest's own
    // error would name the URL, key and all.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    drop(listener);
    let error = Exa::at(http(), url, Some(KEY.into()))
        .search("q", 5)
        .await
        .unwrap_err();
    assert_eq!(error.message, "exa: request failed: transport error");
    assert!(!error.message.contains(KEY));
}

// Keenable ------------------------------------------------------------------

fn keenable_body() -> String {
    let page = "word ".repeat(400);
    json!({"results":[
        {"title":"A","url":"https://a.example","snippet":page,"description":"meta"},
        {"title":"B","url":"https://b.example","snippet":"","description":"only a meta description"},
        {"title":"C","url":"https://c.example"},
    ]})
    .to_string()
}

#[tokio::test]
async fn keenable_keyless_uses_the_public_endpoint() {
    let server = mock(Reply::json(keenable_body())).await;
    let hits = Keenable::at(http(), server.base.clone(), None)
        .search("rust", 2)
        .await
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].snippet.chars().count(), 500);
    assert_eq!(hits[1].snippet, "only a meta description");
    let seen = server.only();
    assert_eq!(seen.path, "/v1/search/public");
    assert!(seen.header("x-api-key").is_none());
    assert_eq!(seen.header("accept"), Some("application/json"));
    assert_eq!(seen.json(), json!({"query":"rust","mode":"pro"}));
}

#[tokio::test]
async fn keenable_with_a_key_uses_the_authenticated_endpoint() {
    let server = mock(Reply::json(keenable_body())).await;
    Keenable::at(http(), server.base.clone(), Some(KEY.into()))
        .search("rust", 3)
        .await
        .unwrap();
    let seen = server.only();
    assert_eq!(seen.path, "/v1/search");
    assert_eq!(seen.header("x-api-key"), Some(KEY));
}

#[tokio::test]
async fn keenable_rate_limit_tells_a_keyless_caller_a_key_lifts_it() {
    let server = mock(Reply::status(429, "")).await;
    let error = Keenable::at(http(), server.base.clone(), None)
        .search("q", 3)
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("keenable: rate limited"),
        "{error}"
    );
    assert!(error.message.contains("add a key"));
}

// You.com -------------------------------------------------------------------

fn youcom_text() -> String {
    json!({"results":{"web":[
        {"title":"Go Docs","url":"https://go.dev/doc/","description":"The Go Programming Language","snippets":["Go is open source"]},
        {"title":"Go Blog","url":"https://go.dev/blog/","description":"","snippets":["The Go Blog","Second"]},
        {"title":"no url","url":"","description":"x","snippets":[]},
    ]}})
    .to_string()
}

#[tokio::test]
async fn youcom_keyless_uses_the_free_profile_and_no_authorization() {
    let server = mock(Reply::sse(Reply::tool_text(&youcom_text()))).await;
    let hits = Youcom::at(http(), server.mcp(), None)
        .search("golang", 5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].snippet, "The Go Programming Language");
    assert_eq!(
        hits[1].snippet, "The Go Blog",
        "the first snippet stands in for a description"
    );
    let seen = server.only();
    assert_eq!(seen.path, "/mcp?profile=free");
    assert!(seen.header("authorization").is_none());
    let body = seen.json();
    assert_eq!(body["params"]["name"], "you-search");
    assert_eq!(body["params"]["arguments"]["query"], "golang");
    assert_eq!(body["params"]["arguments"]["count"], 5);
    assert_eq!(body["params"]["arguments"]["extraction"], "none");
}

#[tokio::test]
async fn youcom_with_a_key_uses_the_keyed_endpoint_and_a_bearer() {
    let server = mock(Reply::sse(Reply::tool_text(&youcom_text()))).await;
    Youcom::at(http(), server.mcp(), Some(KEY.into()))
        .search("golang", 5)
        .await
        .unwrap();
    let seen = server.only();
    assert_eq!(seen.path, "/mcp");
    assert_eq!(
        seen.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
}

#[tokio::test]
async fn youcom_reports_a_rejected_key_that_came_back_as_a_tool_error() {
    let rpc = json!({"result":{"isError":true,"content":[{"type":"text","text":"HTTP 401 unauthorized"}]}});
    let server = mock(Reply::sse(rpc.to_string())).await;
    let error = Youcom::at(http(), server.mcp(), Some(KEY.into()))
        .search("q", 5)
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "youcom: API key rejected (check it in Settings, Tools)"
    );
}

#[tokio::test]
async fn youcom_statuses_name_credits_and_limits() {
    let spent = mock(Reply::status(402, "")).await;
    let error = Youcom::at(http(), spent.mcp(), Some(KEY.into()))
        .search("q", 5)
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("youcom: credits exhausted"),
        "{error}"
    );
    let limited = mock(Reply::status(429, "")).await;
    let error = Youcom::at(http(), limited.mcp(), None)
        .search("q", 5)
        .await
        .unwrap_err();
    assert!(error.message.contains("rate limited"), "{error}");
}

// The chain -----------------------------------------------------------------

/// A provider that answers as scripted and notes that it was asked.
struct Scripted {
    answer: Result<Vec<Hit>, String>,
    delay: Duration,
    asked: Arc<Mutex<Vec<&'static str>>>,
    who: &'static str,
}

#[async_trait]
impl Searcher for Scripted {
    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<Hit>, Failure> {
        self.asked.lock().unwrap().push(self.who);
        tokio::time::sleep(self.delay).await;
        self.answer.clone().map_err(Failure::new)
    }
}

fn hit(title: &str) -> Hit {
    Hit {
        title: title.into(),
        url: format!("https://{title}.example"),
        snippet: String::new(),
    }
}

fn scripted(
    provider: P,
    answer: Result<Vec<Hit>, &str>,
    delay: Duration,
    asked: &Arc<Mutex<Vec<&'static str>>>,
) -> Attempt {
    Attempt {
        provider,
        searcher: Box::new(Scripted {
            answer: answer.map_err(str::to_string),
            delay,
            asked: asked.clone(),
            who: id(provider),
        }),
        key: None,
    }
}

#[tokio::test]
async fn the_chain_tries_in_order_and_stops_at_the_first_success() {
    let asked = Arc::default();
    let chain = Chain::new(vec![
        scripted(
            P::Parallel,
            Err("parallel: rate limited"),
            Duration::ZERO,
            &asked,
        ),
        scripted(P::Exa, Ok(vec![hit("a")]), Duration::ZERO, &asked),
        scripted(P::Keenable, Ok(vec![hit("b")]), Duration::ZERO, &asked),
    ]);
    let answered = chain.search("q", 5).await.unwrap();
    assert_eq!(answered.provider, P::Exa);
    assert_eq!(answered.hits, vec![hit("a")]);
    assert_eq!(*asked.lock().unwrap(), vec!["parallel", "exa"]);
    assert_eq!(
        answered.failed,
        vec![(P::Parallel, "parallel: rate limited".to_string())]
    );
}

#[tokio::test]
async fn an_empty_answer_is_an_answer_and_does_not_fall_through() {
    let asked = Arc::default();
    let chain = Chain::new(vec![
        scripted(P::Parallel, Ok(vec![]), Duration::ZERO, &asked),
        scripted(P::Exa, Ok(vec![hit("a")]), Duration::ZERO, &asked),
    ]);
    let answered = chain.search("q", 5).await.unwrap();
    assert_eq!(answered.provider, P::Parallel);
    assert!(answered.hits.is_empty());
    assert_eq!(*asked.lock().unwrap(), vec!["parallel"]);
    assert!(render(&answered).starts_with("No results."));
}

#[tokio::test]
async fn a_slow_provider_times_out_and_the_next_answers() {
    let asked = Arc::default();
    let chain = Chain::new(vec![
        scripted(
            P::Parallel,
            Ok(vec![hit("slow")]),
            Duration::from_secs(5),
            &asked,
        ),
        scripted(P::Exa, Ok(vec![hit("fast")]), Duration::ZERO, &asked),
    ])
    .with_bounds(Duration::from_millis(60), Duration::from_secs(5));
    let answered = chain.search("q", 5).await.unwrap();
    assert_eq!(answered.provider, P::Exa);
    assert!(
        answered.failed[0].1.contains("timed out"),
        "{:?}",
        answered.failed
    );
}

#[tokio::test]
async fn the_error_names_every_provider_tried_and_why() {
    let asked = Arc::default();
    let chain = Chain::new(vec![
        scripted(
            P::Parallel,
            Err("parallel returned status 503"),
            Duration::ZERO,
            &asked,
        ),
        scripted(P::Exa, Err("exa: rate limited"), Duration::ZERO, &asked),
        scripted(
            P::Keenable,
            Err("keenable: request failed: timed out"),
            Duration::ZERO,
            &asked,
        ),
        scripted(
            P::Youcom,
            Err("youcom returned status 500"),
            Duration::ZERO,
            &asked,
        ),
    ]);
    let error = chain.search("q", 5).await.unwrap_err();
    assert_eq!(
        error,
        "all 4 providers failed (parallel returned status 503; exa: rate limited; keenable: request failed: timed out; youcom returned status 500)"
    );
}

#[tokio::test]
async fn the_total_budget_stops_the_chain_and_names_who_failed_before_it() {
    let asked = Arc::default();
    let slow = Duration::from_millis(200);
    let chain = Chain::new(vec![
        scripted(
            P::Parallel,
            Err("parallel returned status 500"),
            Duration::ZERO,
            &asked,
        ),
        scripted(P::Exa, Ok(vec![hit("x")]), slow, &asked),
        scripted(P::Keenable, Ok(vec![hit("y")]), slow, &asked),
        scripted(P::Youcom, Ok(vec![hit("z")]), slow, &asked),
    ])
    .with_bounds(Duration::from_millis(100), Duration::from_millis(150));
    let started = Instant::now();
    let error = chain.search("q", 5).await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );
    assert!(error.contains("exceeded its 150ms budget after"), "{error}");
    assert!(error.contains("parallel returned status 500"), "{error}");
    assert!(error.contains("exa: timed out"), "{error}");
    assert!(
        !asked.lock().unwrap().contains(&"youcom"),
        "the budget spent, nothing else is tried"
    );
}

#[tokio::test]
async fn a_key_is_scrubbed_from_whatever_a_provider_says() {
    let asked = Arc::default();
    let mut attempt = scripted(
        P::Exa,
        Err(&format!("exa returned status 400: bad key {KEY}")),
        Duration::ZERO,
        &asked,
    );
    attempt.key = Some(KEY.into());
    let error = Chain::new(vec![attempt]).search("q", 5).await.unwrap_err();
    assert!(!error.contains(KEY), "{error}");
    assert!(error.contains("[key]"));
}

/// Real providers over mock servers: a 429, a 500, a hang, then an answer.
#[tokio::test]
async fn the_chain_falls_through_429_5xx_and_a_hang_to_the_provider_that_answers() {
    let limited = mock(Reply::status(429, "")).await;
    let broken = mock(Reply::status(503, "unavailable")).await;
    let hung = mock(Reply::json("{}").after(Duration::from_secs(5))).await;
    let youcom_payload =
        json!({"results":{"web":[{"title":"T","url":"https://t.example","description":"d"}]}});
    let good = mock(Reply::sse(Reply::tool_text(&youcom_payload.to_string()))).await;
    let client = http();
    let chain = Chain::new(vec![
        attempt(
            P::Parallel,
            &client,
            None,
            &Endpoints {
                parallel: limited.mcp(),
                ..Endpoints::default()
            },
        ),
        attempt(
            P::Exa,
            &client,
            None,
            &Endpoints {
                exa: broken.mcp(),
                ..Endpoints::default()
            },
        ),
        attempt(
            P::Keenable,
            &client,
            None,
            &Endpoints {
                keenable: hung.base.clone(),
                ..Endpoints::default()
            },
        ),
        attempt(
            P::Youcom,
            &client,
            None,
            &Endpoints {
                youcom: good.mcp(),
                ..Endpoints::default()
            },
        ),
    ])
    .with_bounds(Duration::from_millis(300), Duration::from_secs(5));
    let answered = chain.search("q", 5).await.unwrap();
    assert_eq!(answered.provider, P::Youcom);
    let reasons: Vec<&str> = answered
        .failed
        .iter()
        .map(|(_, why)| why.as_str())
        .collect();
    assert!(
        reasons[0].starts_with("parallel: rate limited"),
        "{reasons:?}"
    );
    assert!(
        reasons[1].starts_with("exa returned status 503"),
        "{reasons:?}"
    );
    assert!(reasons[2].contains("timed out"), "{reasons:?}");
    assert_eq!(
        render(&answered),
        "1. T\n   https://t.example\n   d\n\nSearched with You.com."
    );
}

// Who is on the chain -------------------------------------------------------

fn set(providers: &[P]) -> HashSet<P> {
    providers.iter().copied().collect()
}

fn policy(mode: PolicyMode, providers: &[P]) -> WebSearchPolicy {
    WebSearchPolicy {
        mode,
        providers: providers.to_vec(),
    }
}

#[test]
fn every_provider_is_on_by_default_in_ketchs_order() {
    assert_eq!(
        effective_chain(&set(&[]), None, &set(&[])),
        vec![P::Parallel, P::Exa, P::Keenable, P::Youcom]
    );
}

#[test]
fn a_key_moves_a_provider_ahead_of_the_keyless_ones() {
    assert_eq!(
        effective_chain(&set(&[]), None, &set(&[P::Youcom, P::Exa])),
        vec![P::Exa, P::Youcom, P::Parallel, P::Keenable]
    );
}

#[test]
fn the_desk_switch_wins_over_a_teammates_policy_and_a_key() {
    let all = policy(PolicyMode::All, &[]);
    assert_eq!(
        effective_chain(&set(&[P::Exa]), Some(&all), &set(&[P::Exa])),
        vec![P::Parallel, P::Keenable, P::Youcom]
    );
    let wants_exa = policy(PolicyMode::Some, &[P::Exa, P::Keenable]);
    assert_eq!(
        effective_chain(&set(&[P::Exa]), Some(&wants_exa), &set(&[])),
        vec![P::Keenable]
    );
}

#[test]
fn a_teammates_some_narrows_the_chain_and_none_empties_it() {
    let some = policy(PolicyMode::Some, &[P::Youcom, P::Parallel]);
    assert_eq!(
        effective_chain(&set(&[]), Some(&some), &set(&[])),
        vec![P::Parallel, P::Youcom]
    );
    let none = policy(PolicyMode::None, &[P::Exa]);
    assert!(effective_chain(&set(&[]), Some(&none), &set(&[])).is_empty());
    let nothing_named = policy(PolicyMode::Some, &[]);
    assert!(effective_chain(&set(&[]), Some(&nothing_named), &set(&[])).is_empty());
}

#[test]
fn the_desk_setting_reads_leniently_and_is_written_in_one_shape() {
    let settings = json!({"webSearch": {"disabled": ["exa", "firecrawl", 7]}});
    assert_eq!(
        disabled_on_desk(settings.as_object().unwrap()),
        set(&[P::Exa])
    );
    assert!(disabled_on_desk(&serde_json::Map::new()).is_empty());

    assert_eq!(
        normalize_setting(&json!({"disabled": ["youcom", "parallel", "youcom"]})).unwrap(),
        json!({"disabled": ["parallel", "youcom"]})
    );
    assert_eq!(
        normalize_setting(&json!({})).unwrap(),
        json!({"disabled": []})
    );
    for bad in [
        json!([]),
        json!({"disabled": "exa"}),
        json!({"disabled": ["brave"]}),
        json!({"other": 1}),
    ] {
        assert!(normalize_setting(&bad).is_err(), "{bad}");
    }
}

// What the model reads ------------------------------------------------------

#[test]
fn the_answer_is_title_link_snippet_and_the_provider_that_answered() {
    let answered = Answered {
        provider: P::Keenable,
        hits: vec![
            Hit {
                title: "One".into(),
                url: "https://one.example".into(),
                snippet: format!("  line\none   {} ", "x".repeat(400)),
            },
            hit("two"),
        ],
        failed: vec![],
    };
    let text = render(&answered);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "1. One");
    assert_eq!(lines[1], "   https://one.example");
    assert!(lines[2].starts_with("   line one xxx"));
    assert_eq!(lines[2].trim().chars().count(), 300);
    assert!(lines[2].ends_with('…'));
    assert_eq!(lines[4], "2. two");
    assert_eq!(*lines.last().unwrap(), "Searched with Keenable.");
}

/// One real query through each provider, keyless, over the real network:
/// `cargo test -p hotline-core websearch_live -- --ignored --nocapture`.
/// Not run with the suite. A provider that is rate limited or down fails here
/// on its own reason, which is the point of seeing them one at a time.
#[tokio::test]
#[ignore = "reaches the live providers"]
async fn websearch_live_each_provider_keyless() {
    let client = client();
    let endpoints = Endpoints::default();
    let mut failed = Vec::new();
    for provider in ORDER {
        let one = attempt(provider, &client, None, &endpoints);
        let started = Instant::now();
        match tokio::time::timeout(
            ATTEMPT_TIMEOUT,
            one.searcher.search("rust programming language", 5),
        )
        .await
        {
            Ok(Ok(hits)) => {
                println!(
                    "{}: {} results in {:?}; first: {:?}",
                    id(provider),
                    hits.len(),
                    started.elapsed(),
                    hits.first().map(|hit| (&hit.title, &hit.url))
                );
                if hits.is_empty() {
                    failed.push(format!("{}: no results", id(provider)));
                }
            }
            Ok(Err(error)) => {
                println!("{}: FAILED {error}", id(provider));
                failed.push(format!("{}: {error}", id(provider)));
            }
            Err(_) => {
                println!("{}: FAILED timed out", id(provider));
                failed.push(format!("{}: timed out", id(provider)));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}
