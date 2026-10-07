//! The providers against mock servers on localhost, and the chain over them.
//! No test here reaches the network.

use super::*;
use crate::contract::{PolicyMode, WebSearchPolicy, WebSearchProvider as P};
use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    let hits = parallel
        .search(&Request::plain("go context cancellation", 5))
        .await
        .unwrap();

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
    let hits = parallel.search(&Request::plain("q", 1)).await.unwrap();
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
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert_eq!(error.message, "parallel response contained no text results");

    let limited = mock(Reply::status(429, "slow down")).await;
    let error = Parallel::at(http(), limited.mcp(), None)
        .search(&Request::plain("q", 5))
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
        .search(&Request::plain("rust", 5))
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
    assert_eq!(arguments["objective"], "rust");
    assert_eq!(
        arguments.as_object().unwrap().len(),
        3,
        "only what the server's schema has"
    );
}

#[tokio::test]
async fn exa_passes_a_key_in_the_query_and_respects_the_limit() {
    let server = mock(Reply::sse(Reply::tool_text(EXA_TEXT))).await;
    let hits = Exa::at(http(), server.mcp(), Some(KEY.into()))
        .search(&Request::plain("rust", 1))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(server.only().path, format!("/mcp?exaApiKey={KEY}"));
}

#[tokio::test]
async fn exa_with_nothing_matched_is_an_empty_success() {
    let server = mock(Reply::sse(json!({"result":{"content":[]}}).to_string())).await;
    let hits = Exa::at(http(), server.mcp(), None)
        .search(&Request::plain("zzz", 5))
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn exa_statuses_read_as_a_person_would_and_never_carry_the_key() {
    let denied = mock(Reply::status(401, "")).await;
    let error = Exa::at(http(), denied.mcp(), Some(KEY.into()))
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert!(error.message.contains("invalid API key"), "{error}");
    assert!(!error.message.contains(KEY));

    let broken = mock(Reply::status(500, "boom")).await;
    let error = Exa::at(http(), broken.mcp(), None)
        .search(&Request::plain("q", 5))
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
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert_eq!(error.message, "exa: request failed: transport error");
    assert!(!error.message.contains(KEY));
}

// Keenable ------------------------------------------------------------------

fn keenable_body() -> String {
    let page = "word ".repeat(400);
    json!({"results":[
        {"title":"A","url":"https://a.example","snippet":page,"description":""},
        {"title":"B","url":"https://b.example","snippet":"","description":"only a meta description"},
        {"title":"C","url":"https://c.example"},
    ]})
    .to_string()
}

#[tokio::test]
async fn keenable_keyless_uses_the_public_endpoint() {
    let server = mock(Reply::json(keenable_body())).await;
    let hits = Keenable::at(http(), server.base.clone(), None)
        .search(&Request::plain("rust", 2))
        .await
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].snippet.chars().count(), 500);
    assert_eq!(hits[1].snippet, "only a meta description");
    let seen = server.only();
    assert_eq!(seen.path, "/v1/search/public");
    assert!(seen.header("x-api-key").is_none());
    assert_eq!(seen.header("accept"), Some("application/json"));
    assert_eq!(
        seen.json(),
        json!({"query":"rust","mode":"pro","max_results":2})
    );
}

#[tokio::test]
async fn keenable_with_a_key_uses_the_authenticated_endpoint() {
    let server = mock(Reply::json(keenable_body())).await;
    Keenable::at(http(), server.base.clone(), Some(KEY.into()))
        .search(&Request::plain("rust", 3))
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
        .search(&Request::plain("q", 3))
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("keenable: rate limited"),
        "{error}"
    );
    assert!(error.message.contains("add a key"));
}

// Firecrawl -----------------------------------------------------------------

fn firecrawl_body() -> String {
    json!({"success":true,"data":{"web":[
        {"title":"Go Docs","url":"https://go.dev/doc/","description":"The Go Programming Language"},
        {"title":"no url","url":"","description":"x"},
        {"title":"Go Blog","url":"https://go.dev/blog/","description":"The Go Blog"},
    ]}})
    .to_string()
}

#[tokio::test]
async fn firecrawl_keyless_posts_the_v2_search_with_no_authorization() {
    let server = mock(Reply::json(firecrawl_body())).await;
    let hits = Firecrawl::at(http(), server.base.clone(), None)
        .search(&Request::plain("golang", 5))
        .await
        .unwrap();
    assert_eq!(
        hits,
        vec![
            Hit {
                title: "Go Docs".into(),
                url: "https://go.dev/doc/".into(),
                snippet: "The Go Programming Language".into()
            },
            Hit {
                title: "Go Blog".into(),
                url: "https://go.dev/blog/".into(),
                snippet: "The Go Blog".into()
            },
        ]
    );
    let seen = server.only();
    assert_eq!(seen.path, "/v2/search");
    assert!(seen.header("authorization").is_none());
    let body = seen.json();
    assert_eq!(body["query"], "golang");
    assert_eq!(body["limit"], 5);
}

#[tokio::test]
async fn firecrawl_with_a_key_sends_a_bearer_and_respects_the_limit() {
    let server = mock(Reply::json(firecrawl_body())).await;
    let hits = Firecrawl::at(http(), server.base.clone(), Some(KEY.into()))
        .search(&Request::plain("golang", 1))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let seen = server.only();
    assert_eq!(
        seen.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
}

#[tokio::test]
async fn firecrawl_statuses_name_credits_limits_and_bad_keys() {
    let spent = mock(Reply::status(402, "")).await;
    let error = Firecrawl::at(http(), spent.base.clone(), Some(KEY.into()))
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert!(
        error.message.starts_with("firecrawl: credits exhausted"),
        "{error}"
    );
    let limited = mock(Reply::status(429, "")).await;
    let error = Firecrawl::at(http(), limited.base.clone(), None)
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert!(error.message.contains("rate limited"), "{error}");
    assert!(error.message.contains("add a key"), "{error}");
    let denied = mock(Reply::status(401, "")).await;
    let error = Firecrawl::at(http(), denied.base.clone(), Some(KEY.into()))
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert!(error.message.contains("invalid API key"), "{error}");
    assert!(!error.message.contains(KEY));
    let broken = mock(Reply::status(500, "boom")).await;
    let error = Firecrawl::at(http(), broken.base.clone(), None)
        .search(&Request::plain("q", 5))
        .await
        .unwrap_err();
    assert_eq!(error.message, "firecrawl returned status 500: boom");
}

// Language and recency, where each provider takes them ------------------------

fn french_news() -> Request {
    Request {
        query: "actualités".into(),
        depth: 10,
        language: query::language_by_code("fr"),
        recency: true,
    }
}

#[tokio::test]
async fn parallel_and_exa_carry_language_and_recency_in_their_objective_sentence() {
    let server = mock(Reply::json(Reply::tool_text("{\"results\":[]}"))).await;
    let _ = Parallel::at(http(), server.mcp(), None)
        .search(&french_news())
        .await;
    let objective = server.only().json()["params"]["arguments"]["objective"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(objective.starts_with("actualités"), "{objective}");
    assert!(objective.contains("written in French"), "{objective}");
    assert!(objective.contains("recent news"), "{objective}");
    let exa = mock(Reply::sse(Reply::tool_text(EXA_TEXT))).await;
    let _ = Exa::at(http(), exa.mcp(), None)
        .search(&french_news())
        .await;
    let args = exa.only().json()["params"]["arguments"].clone();
    assert!(
        args["objective"]
            .as_str()
            .unwrap()
            .contains("written in French")
    );
    assert_eq!(args["numResults"], 10);
}

#[tokio::test]
async fn firecrawl_takes_a_language_and_a_news_window_and_reads_news_results() {
    let body = json!({"success":true,"data":{
        "web":[{"title":"W","url":"https://w.example","description":"web"}],
        "news":[{"title":"N","url":"https://n.example","snippet":"## story text"}],
    }});
    let server = mock(Reply::json(body.to_string())).await;
    let hits = Firecrawl::at(http(), server.base.clone(), None)
        .search(&french_news())
        .await
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].snippet, "## story text");
    let sent = server.only().json();
    assert_eq!(sent["lang"], "fr");
    assert_eq!(sent["sources"], json!(["web", "news"]));
    assert_eq!(sent["tbs"], "qdr:w");
    assert_eq!(sent["limit"], 10);
    // A plain English query sends neither.
    let plain = mock(Reply::json(firecrawl_body())).await;
    Firecrawl::at(http(), plain.base.clone(), None)
        .search(&Request::plain("rust", 10))
        .await
        .unwrap();
    let sent = plain.only().json();
    assert!(
        sent.get("lang").is_none() && sent.get("tbs").is_none() && sent.get("sources").is_none()
    );
}

#[tokio::test]
async fn keenable_takes_a_depth_and_a_publication_date_and_no_language() {
    let server = mock(Reply::json(keenable_body())).await;
    Keenable::at(http(), server.base.clone(), None)
        .search(&french_news())
        .await
        .unwrap();
    let sent = server.only().json();
    assert_eq!(sent["max_results"], 10);
    assert!(sent["published_after"].as_str().unwrap().len() == 10);
    assert!(sent.get("language").is_none() && sent.get("lang").is_none());
    let plain = mock(Reply::json(keenable_body())).await;
    Keenable::at(http(), plain.base.clone(), None)
        .search(&Request::plain("rust", 12))
        .await
        .unwrap();
    let sent = plain.only().json();
    assert_eq!(sent["max_results"], 12);
    assert!(sent.get("published_after").is_none());
}

#[tokio::test]
async fn keenable_prefers_its_short_description_to_page_text() {
    let body = json!({"results":[{"title":"A","url":"https://a.example","snippet":"Home | Menu | Sign in","description":"A real summary of the page."}]});
    let server = mock(Reply::json(body.to_string())).await;
    let hits = Keenable::at(http(), server.base.clone(), None)
        .search(&Request::plain("q", 5))
        .await
        .unwrap();
    assert_eq!(hits[0].snippet, "A real summary of the page.");
}

// The fan-out ---------------------------------------------------------------

/// A provider that answers as scripted and notes that it was asked.
struct Scripted {
    answer: Result<Vec<Hit>, String>,
    delay: Duration,
    asked: Arc<Mutex<Vec<&'static str>>>,
    who: &'static str,
}

#[async_trait]
impl Searcher for Scripted {
    async fn search(&self, _request: &Request) -> Result<Vec<Hit>, Failure> {
        self.asked.lock().unwrap().push(self.who);
        tokio::time::sleep(self.delay).await;
        self.answer.clone().map_err(Failure::new)
    }
}

fn hit(title: &str) -> Hit {
    Hit {
        title: title.into(),
        url: format!("https://{title}.example/page"),
        snippet: format!("{title} is a page with a sentence long enough to read as real text."),
    }
}

type Asked = Arc<Mutex<Vec<&'static str>>>;

fn scripted(
    provider: P,
    answer: Result<Vec<Hit>, &str>,
    delay: Duration,
    asked: &Asked,
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

fn ask(attempts: Vec<Attempt>) -> Search {
    Search::new(attempts)
}

#[tokio::test]
async fn every_provider_is_asked_and_their_lists_are_fused() {
    let asked = Asked::default();
    let search = ask(vec![
        scripted(
            P::Parallel,
            Ok(vec![hit("alpha"), hit("beta")]),
            Duration::ZERO,
            &asked,
        ),
        scripted(
            P::Exa,
            Ok(vec![hit("gamma"), hit("beta")]),
            Duration::ZERO,
            &asked,
        ),
        scripted(P::Keenable, Ok(vec![hit("delta")]), Duration::ZERO, &asked),
    ]);
    let answered = search.run(&Query::parse("subject"), 5).await.unwrap();
    let mut who = asked.lock().unwrap().clone();
    who.sort();
    assert_eq!(who, vec!["exa", "keenable", "parallel"]);
    let titles: Vec<&str> = answered.hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles[0], "beta", "found by two providers: {titles:?}");
    assert_eq!(titles.len(), 4);
    assert_eq!(answered.answered, vec![P::Parallel, P::Exa, P::Keenable]);
    assert_eq!(
        render(&answered).lines().last().unwrap(),
        "Searched with Parallel, Exa, Keenable."
    );
}

#[tokio::test]
async fn providers_run_at_once_not_one_after_another() {
    let asked = Asked::default();
    let slow = Duration::from_millis(150);
    let search = ask(vec![
        scripted(P::Parallel, Ok(vec![hit("a")]), slow, &asked),
        scripted(P::Exa, Ok(vec![hit("b")]), slow, &asked),
        scripted(P::Keenable, Ok(vec![hit("c")]), slow, &asked),
        scripted(P::Firecrawl, Ok(vec![hit("d")]), slow, &asked),
    ]);
    let started = Instant::now();
    let answered = search.run(&Query::parse("subject"), 5).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(answered.hits.len(), 4);
}

#[tokio::test]
async fn a_failed_or_slow_provider_costs_its_list_and_is_named_in_the_footer() {
    let asked = Asked::default();
    let search = ask(vec![
        scripted(P::Parallel, Ok(vec![hit("a")]), Duration::ZERO, &asked),
        scripted(P::Exa, Err("exa: rate limited"), Duration::ZERO, &asked),
        scripted(P::Keenable, Ok(vec![hit("b")]), Duration::ZERO, &asked),
        scripted(
            P::Firecrawl,
            Ok(vec![hit("slow")]),
            Duration::from_secs(5),
            &asked,
        ),
    ])
    .with_bounds(Duration::from_millis(80), Duration::from_secs(2));
    let answered = search.run(&Query::parse("subject"), 5).await.unwrap();
    assert_eq!(answered.hits.len(), 2);
    assert_eq!(
        render(&answered).lines().last().unwrap(),
        "Searched with Parallel, Keenable (Exa rate limited, Firecrawl timed out)."
    );
}

#[tokio::test]
async fn the_total_budget_cuts_the_wait_and_uses_what_arrived() {
    let asked = Asked::default();
    let search = ask(vec![
        scripted(P::Parallel, Ok(vec![hit("quick")]), Duration::ZERO, &asked),
        scripted(
            P::Exa,
            Ok(vec![hit("late")]),
            Duration::from_secs(5),
            &asked,
        ),
    ])
    .with_bounds(Duration::from_secs(5), Duration::from_millis(120));
    let started = Instant::now();
    let answered = search.run(&Query::parse("subject"), 5).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(answered.answered, vec![P::Parallel]);
    assert!(
        answered.failed[0].1.contains("timed out after 120ms"),
        "{:?}",
        answered.failed
    );
}

#[tokio::test]
async fn when_nothing_arrives_by_the_budget_the_error_says_so() {
    let asked = Asked::default();
    let search = ask(vec![
        scripted(
            P::Parallel,
            Ok(vec![hit("a")]),
            Duration::from_secs(5),
            &asked,
        ),
        scripted(P::Exa, Ok(vec![hit("b")]), Duration::from_secs(5), &asked),
    ])
    .with_bounds(Duration::from_secs(5), Duration::from_millis(100));
    let error = search.run(&Query::parse("subject"), 5).await.unwrap_err();
    assert!(
        error.starts_with("web search exceeded its 100ms budget before any provider answered"),
        "{error}"
    );
    assert!(
        error.contains("parallel: timed out") && error.contains("exa: timed out"),
        "{error}"
    );
}

#[tokio::test]
async fn the_error_names_every_provider_tried_and_why() {
    let asked = Asked::default();
    let search = ask(vec![
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
            P::Firecrawl,
            Err("firecrawl returned status 500"),
            Duration::ZERO,
            &asked,
        ),
    ]);
    let error = search.run(&Query::parse("subject"), 5).await.unwrap_err();
    assert_eq!(
        error,
        "all 4 providers failed (parallel returned status 503; exa: rate limited; keenable: request failed: timed out; firecrawl returned status 500)"
    );
}

#[tokio::test]
async fn an_empty_answer_is_an_answer() {
    let asked = Asked::default();
    let search = ask(vec![scripted(
        P::Parallel,
        Ok(vec![]),
        Duration::ZERO,
        &asked,
    )]);
    let answered = search.run(&Query::parse("subject"), 5).await.unwrap();
    assert!(render(&answered).starts_with("No results."));
}

#[tokio::test]
async fn a_key_is_scrubbed_from_whatever_a_provider_says() {
    let asked = Asked::default();
    let mut attempt = scripted(
        P::Exa,
        Err(&format!("exa returned status 400: bad key {KEY}")),
        Duration::ZERO,
        &asked,
    );
    attempt.key = Some(KEY.into());
    let error = ask(vec![attempt])
        .run(&Query::parse("subject"), 5)
        .await
        .unwrap_err();
    assert!(!error.contains(KEY), "{error}");
    assert!(error.contains("[key]"));
}

/// Real providers over mock servers: a 429, a 503, a hang, then an answer.
#[tokio::test]
async fn real_providers_over_mocks_fall_out_of_the_fusion_without_costing_the_answer() {
    let limited = mock(Reply::status(429, "")).await;
    let broken = mock(Reply::status(503, "unavailable")).await;
    let hung = mock(Reply::json("{}").after(Duration::from_secs(5))).await;
    let payload = json!({"data":{"web":[{"title":"T","url":"https://t.example","description":"A page that says what it is in one full sentence."}]}});
    let good = mock(Reply::json(payload.to_string())).await;
    let client = http();
    let search = Search::new(vec![
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
            P::Firecrawl,
            &client,
            None,
            &Endpoints {
                firecrawl: good.base.clone(),
                ..Endpoints::default()
            },
        ),
    ])
    .with_bounds(Duration::from_millis(300), Duration::from_secs(5));
    let answered = search.run(&Query::parse("sentence"), 5).await.unwrap();
    assert_eq!(answered.answered, vec![P::Firecrawl]);
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
        "1. T\n   https://t.example\n   A page that says what it is in one full sentence.\n\nSearched with Firecrawl (Parallel rate limited, Exa failed, Keenable timed out)."
    );
}

#[test]
fn the_note_comes_first_when_the_rare_words_matched_nothing() {
    let answered = Answered {
        hits: vec![hit("a")],
        note: Some("No result mentions \"zzzqqq\".".into()),
        answered: vec![P::Exa],
        failed: vec![],
    };
    assert!(render(&answered).starts_with("No result mentions \"zzzqqq\".\n\n1. a"));
}

#[test]
fn depth_asks_for_more_than_the_limit_within_bounds() {
    assert_eq!(
        (depth_for(1), depth_for(8), depth_for(12), depth_for(20)),
        (10, 16, 20, 20)
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
        effective_chain(&set(&[]), None),
        vec![P::Parallel, P::Exa, P::Keenable, P::Firecrawl]
    );
}

#[test]
fn the_desk_switch_wins_over_a_teammates_policy() {
    let all = policy(PolicyMode::All, &[]);
    assert_eq!(
        effective_chain(&set(&[P::Exa]), Some(&all)),
        vec![P::Parallel, P::Keenable, P::Firecrawl]
    );
    let wants_exa = policy(PolicyMode::Some, &[P::Exa, P::Keenable]);
    assert_eq!(
        effective_chain(&set(&[P::Exa]), Some(&wants_exa)),
        vec![P::Keenable]
    );
}

#[test]
fn a_teammates_some_narrows_the_chain_and_none_empties_it() {
    let some = policy(PolicyMode::Some, &[P::Firecrawl, P::Parallel]);
    assert_eq!(
        effective_chain(&set(&[]), Some(&some)),
        vec![P::Parallel, P::Firecrawl]
    );
    let none = policy(PolicyMode::None, &[P::Exa]);
    assert!(effective_chain(&set(&[]), Some(&none)).is_empty());
    let nothing_named = policy(PolicyMode::Some, &[]);
    assert!(effective_chain(&set(&[]), Some(&nothing_named)).is_empty());
}

#[test]
fn the_desk_setting_reads_leniently_and_is_written_in_one_shape() {
    let settings = json!({"webSearch": {"disabled": ["exa", "youcom", 7]}});
    assert_eq!(
        disabled_on_desk(settings.as_object().unwrap()),
        set(&[P::Exa])
    );
    assert!(disabled_on_desk(&serde_json::Map::new()).is_empty());

    assert_eq!(
        normalize_setting(&json!({"disabled": ["firecrawl", "parallel", "firecrawl"]})).unwrap(),
        json!({"disabled": ["parallel", "firecrawl"]})
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
fn the_answer_is_title_link_snippet_and_who_answered() {
    let answered = Answered {
        hits: vec![
            Hit {
                title: "One".into(),
                url: "https://one.example".into(),
                snippet: format!("  line\none   {} ", "x".repeat(400)),
            },
            Hit {
                title: "two".into(),
                url: "https://two.example".into(),
                snippet: String::new(),
            },
        ],
        note: None,
        answered: vec![P::Keenable],
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

// Live ----------------------------------------------------------------------

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
        let request = Request::plain("rust programming language", 5);
        match tokio::time::timeout(ATTEMPT_TIMEOUT, one.searcher.search(&request)).await {
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

/// The quality regression: each query that went wrong live, through the whole
/// pipeline, printing the top five with their sources, and soft checks on
/// what George found. `cargo test -p hotline-core websearch_quality_live --
/// --ignored --nocapture`.
#[tokio::test]
#[ignore = "reaches the live providers"]
async fn websearch_quality_live() {
    let client = client();
    let endpoints = Endpoints::default();
    let mut problems: Vec<String> = Vec::new();
    for text in [
        "Mount Everest",
        "Mont Everest altitude",
        "apple",
        "hotline.dev desktop app",
        "tokio vs async-std",
        "latest world news October 6 2026",
        "why is the sky blue",
        "RFC 9110 HTTP semantics",
        "xqzflarnib 98421 protocol",
    ] {
        let query = Query::parse(text);
        let search = Search::new(
            ORDER
                .into_iter()
                .map(|provider| attempt(provider, &client, None, &endpoints))
                .collect(),
        );
        let started = Instant::now();
        let answered = match search.run(&query, 5).await {
            Ok(answered) => answered,
            Err(error) => {
                println!("\n== {text}\n   FAILED {error}");
                problems.push(format!("{text}: {error}"));
                continue;
            }
        };
        println!(
            "\n== {text}  [{:?}, language {:?}]\n{}",
            started.elapsed(),
            query.language.map(|l| l.code),
            render(&answered)
        );
        problems.extend(quality_problems(text, &answered));
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

fn quality_problems(text: &str, answered: &Answered) -> Vec<String> {
    let mut problems = Vec::new();
    let top = |n: usize| answered.hits.iter().take(n).collect::<Vec<_>>();
    let host_has = |hit: &Hit, part: &str| hit.url.to_lowercase().contains(part);
    let mut keys = HashSet::new();
    for hit in &answered.hits {
        if !keys.insert(canonical::canonical_url(&hit.url)) {
            problems.push(format!("{text}: duplicate url {}", hit.url));
        }
        if hit.snippet.contains("Moved Permanently") {
            problems.push(format!("{text}: error-page snippet on {}", hit.url));
        }
    }
    let mirrors: Vec<Option<String>> = answered.hits.iter().map(|h| rank_mirror(&h.url)).collect();
    let distinct: HashSet<_> = mirrors.iter().flatten().collect();
    if distinct.len() < mirrors.iter().flatten().count() {
        problems.push(format!("{text}: a mirror pair in the top 5"));
    }
    match text {
        "Mount Everest" => {
            if !top(3)
                .iter()
                .any(|h| host_has(h, "en.wikipedia") || host_has(h, "kathmandupost"))
            {
                problems.push(format!(
                    "{text}: neither en.wikipedia nor kathmandupost in the top 3"
                ));
            }
            if top(3).iter().any(|h| host_has(h, "facebook")) {
                problems.push(format!("{text}: facebook in the top 3"));
            }
        }
        "Mont Everest altitude" => {
            let french = top(3)
                .iter()
                .filter(|h| {
                    query::text_language(&format!("{}. {}", h.title, h.snippet))
                        .is_some_and(|l| l.code == "fr")
                        || host_has(h, "fr.wikipedia")
                })
                .count();
            if french == 0 {
                problems.push(format!("{text}: no French result in the top 3"));
            }
        }
        "apple" => {
            if !top(2).iter().any(|h| host_has(h, "apple.com")) {
                problems.push(format!("{text}: apple.com not in the top 2"));
            }
        }
        "hotline.dev desktop app"
            if !answered
                .hits
                .first()
                .is_some_and(|h| host_has(h, "hotline.dev")) =>
        {
            problems.push(format!("{text}: hotline.dev is not first"));
        }
        "xqzflarnib 98421 protocol"
            if !answered
                .note
                .as_deref()
                .is_some_and(|n| n.contains("xqzflarnib")) =>
        {
            problems.push(format!("{text}: no missed-word note"));
        }
        _ => {}
    }
    problems
}

/// The mirror key the ranking uses, for the check that none survive.
fn rank_mirror(url: &str) -> Option<String> {
    rank::mirror_key_for_tests(url)
}
