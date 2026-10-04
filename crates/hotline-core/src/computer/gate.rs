//! One teammate's computer, driven by one thread at a time.
//!
//! A teammate works in several threads at once: its DM, work threads, a
//! colleague's ask. Every one of them is handed the teammate's one computer,
//! and two agents driving one desktop is a fight nobody wins. So an agent is
//! not handed the computer's own endpoint but a gate in front of it ([`serve`]),
//! one per agent, which forwards everything and lets only the thread that
//! holds the teammate's lease make a `tools/call`.
//!
//! The lease is [`Leases`]. A thread takes it on its first computer call and
//! keeps it while it is using the computer: it is let go of when the thread's
//! turn ends ([`Leases::release`]), when the thread closes, parks or is stopped
//! (the gate ends with the thread's authority and releases what it held), and
//! when the thread has made no call for [`Timing::idle`], so a thread that has
//! simply stopped needing the machine does not hold it. A call in flight is
//! never taken from under it, and a release while one is in flight takes
//! effect when it ends, so the computer is never driven by two at once. A
//! lease belongs to one agent, not to a thread's name: a replacement agent of
//! the same thread is a different [`Holder`], and the one it replaces cannot
//! let go of what it holds. Another thread's call waits up to
//! [`Timing::wait`] for the lease, and is then answered, as a tool result and
//! not a protocol error, that the computer is busy and who has it.
//!
//! The gate reaches only the computer it was made for, with the bearer the
//! teammate's grant already carried: it adds no authority, and a thread whose
//! lease has been revoked is refused at the gate before anything is forwarded,
//! including a call that was waiting for the computer when it was revoked.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// How long a thread keeps the computer without using it, and how long another
/// waits for it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    pub idle: Duration,
    pub wait: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(30),
            wait: Duration::from_secs(20),
        }
    }
}

/// Who is asking for the computer: one agent of one thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Holder {
    /// What the lease is kept under: the thread's own name.
    pub key: String,
    /// What another thread is told when it is turned away.
    pub title: String,
    /// Which agent of the thread this is. A thread's agent is replaced while
    /// the one it replaces is still shutting down, and it is this that tells
    /// their holds apart.
    owner: u64,
}

impl Holder {
    /// A holder of its own, never equal to another made for the same thread.
    pub(crate) fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            key: key.into(),
            title: title.into(),
            owner: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
}

struct Held {
    by: Holder,
    last: Instant,
    in_flight: usize,
    /// Let go of while a call was in flight: it is the holder's until the
    /// call ends, and nobody's after.
    released: bool,
}

/// Why a call was not given the computer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Another thread has it; what to tell the caller.
    Busy(String),
    /// The caller's authority ended while it waited.
    Revoked,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(text) => out.write_str(text),
            Self::Revoked => out.write_str("This thread's authority has ended."),
        }
    }
}

/// Who has each teammate's computer.
pub(crate) struct Leases {
    held: Mutex<HashMap<String, Held>>,
    freed: Notify,
    timing: Timing,
}

/// A computer call that has the lease. Dropping it ends the call, which is
/// when its idle clock starts.
pub(crate) struct Call {
    leases: Arc<Leases>,
    persona_id: String,
    holder: Holder,
}

impl Drop for Call {
    fn drop(&mut self) {
        let mut held = self
            .leases
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = held.get_mut(&self.persona_id) else {
            return;
        };
        if entry.by != self.holder {
            return;
        }
        entry.in_flight = entry.in_flight.saturating_sub(1);
        entry.last = Instant::now();
        if entry.released && entry.in_flight == 0 {
            held.remove(&self.persona_id);
            drop(held);
            self.leases.freed.notify_waiters();
        }
    }
}

impl Leases {
    pub(crate) fn new() -> Arc<Self> {
        Self::with(Timing::default())
    }

    pub(crate) fn with(timing: Timing) -> Arc<Self> {
        Arc::new(Self {
            held: Mutex::new(HashMap::new()),
            freed: Notify::new(),
            timing,
        })
    }

    /// Takes the lease for a call, or waits for it, and says who has it when
    /// the wait runs out. `alive` is the caller's authority, asked again
    /// every time the lease could be given, so a caller whose authority ended
    /// while it waited is turned away and never given the computer.
    pub(crate) async fn take(
        self: &Arc<Self>,
        persona_id: &str,
        holder: &Holder,
        alive: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Call, Refused> {
        let deadline = Instant::now() + self.timing.wait;
        loop {
            let freed = self.freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();
            let busy = {
                let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
                if !alive() {
                    return Err(Refused::Revoked);
                }
                match held.get_mut(persona_id) {
                    Some(entry) if entry.by == *holder => {
                        entry.in_flight += 1;
                        entry.released = false;
                        None
                    }
                    Some(entry)
                        if entry.in_flight == 0 && entry.last.elapsed() >= self.timing.idle =>
                    {
                        *entry = Held {
                            by: holder.clone(),
                            last: Instant::now(),
                            in_flight: 1,
                            released: false,
                        };
                        None
                    }
                    Some(entry) => Some(entry.by.title.clone()),
                    None => {
                        held.insert(
                            persona_id.to_string(),
                            Held {
                                by: holder.clone(),
                                last: Instant::now(),
                                in_flight: 1,
                                released: false,
                            },
                        );
                        None
                    }
                }
            };
            let Some(title) = busy else {
                return Ok(Call {
                    leases: self.clone(),
                    persona_id: persona_id.to_string(),
                    holder: holder.clone(),
                });
            };
            let now = Instant::now();
            if now >= deadline {
                return Err(Refused::Busy(format!(
                    "The computer is busy: {title} is using it right now. Work on something that does not need it, or try again in a minute."
                )));
            }
            // Wake when it is let go of, or often enough to see the holder go
            // idle, which nothing announces.
            let _ =
                tokio::time::timeout((deadline - now).min(Duration::from_millis(250)), &mut freed)
                    .await;
        }
    }

    /// Lets go of what `holder` holds of this teammate's computer, if
    /// anything. A call in flight keeps the computer until it ends: the hold
    /// goes then, and nothing else is let on in between.
    pub(crate) fn release(&self, persona_id: &str, holder: &Holder) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = held.get_mut(persona_id) else {
            return;
        };
        if entry.by != *holder {
            return;
        }
        if entry.in_flight > 0 {
            entry.released = true;
            return;
        }
        held.remove(persona_id);
        drop(held);
        self.freed.notify_waiters();
    }

    /// Who has this teammate's computer now, for a test and for the log.
    #[cfg(test)]
    pub(crate) fn holder(&self, persona_id: &str) -> Option<String> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(persona_id)
            .map(|entry| entry.by.key.clone())
    }
}

/// The gate: where an agent is told the computer is, in place of the
/// computer's own endpoint.
pub(crate) struct Gate {
    pub url: String,
    pub token: String,
}

struct Shared {
    client: reqwest::Client,
    origin: String,
    token: String,
    leases: Arc<Leases>,
    persona_id: String,
    holder: Holder,
    alive: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// Puts a gate in front of `upstream` for one agent. The gate ends, and
/// releases what it held, when `alive` says the agent's authority is gone.
pub(crate) async fn serve(
    upstream: &super::Ready,
    leases: Arc<Leases>,
    persona_id: &str,
    holder: Holder,
    alive: impl Fn() -> bool + Send + Sync + 'static,
) -> std::io::Result<Gate> {
    let parsed = url::Url::parse(&upstream.url)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let origin = parsed.origin().ascii_serialization();
    let path = parsed.path().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let alive: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(alive);
    let state = Arc::new(Shared {
        client: reqwest::Client::new(),
        origin,
        token: upstream.token.clone(),
        leases: leases.clone(),
        persona_id: persona_id.to_string(),
        holder: holder.clone(),
        alive: alive.clone(),
    });
    let expected = token.clone();
    let router = axum::Router::new()
        .fallback(forward)
        .with_state(state)
        .layer(axum::middleware::from_fn(
            move |request: Request, next: axum::middleware::Next| {
                let expected = expected.clone();
                async move {
                    let presented = request
                        .headers()
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.strip_prefix("Bearer "))
                        .unwrap_or("");
                    if !crate::wire::same_secret(presented, &expected) {
                        return Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Body::empty())
                            .expect("a bare status is a response");
                    }
                    next.run(request).await
                }
            },
        ));
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let persona = persona_id.to_string();
    tokio::spawn(async move {
        let ended = async move {
            while alive() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        };
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(ended)
            .await;
        leases.release(&persona, &holder);
    });
    Ok(Gate {
        url: format!("http://127.0.0.1:{port}{path}"),
        token,
    })
}

/// Headers that belong to one hop, or that the other end sets itself.
fn per_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "authorization"
            | "connection"
            | "transfer-encoding"
            | "accept-encoding"
            | "keep-alive"
            | "upgrade"
    )
}

/// What a request calls, when it calls a tool.
fn calls_a_tool(body: &[u8]) -> Option<Value> {
    let message: Value = serde_json::from_slice(body).ok()?;
    let calls = |one: &Value| one.get("method").and_then(Value::as_str) == Some("tools/call");
    match &message {
        Value::Array(many) if many.iter().any(calls) => Some(Value::Null),
        one if calls(one) => Some(one.get("id").cloned().unwrap_or(Value::Null)),
        _ => None,
    }
}

fn answer(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("a json answer is a response")
}

fn ended() -> Response {
    answer(
        StatusCode::FORBIDDEN,
        json!({ "error": "This thread's authority has ended." }),
    )
}

async fn forward(State(state): State<Arc<Shared>>, request: Request) -> Response {
    if !(state.alive)() {
        return ended();
    }
    let (parts, body) = request.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 64 * 1024 * 1024).await else {
        return answer(
            StatusCode::PAYLOAD_TOO_LARGE,
            json!({ "error": "Too large." }),
        );
    };
    let call = if parts.method == Method::POST {
        match calls_a_tool(&bytes) {
            Some(id) => match state
                .leases
                .take(&state.persona_id, &state.holder, &*state.alive)
                .await
            {
                Ok(call) => Some(call),
                Err(Refused::Revoked) => return ended(),
                Err(busy) => {
                    return answer(
                        StatusCode::OK,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{ "type": "text", "text": busy.to_string() }],
                                "isError": true,
                            },
                        }),
                    );
                }
            },
            None => None,
        }
    } else {
        None
    };
    // The authority is checked again with the lease in hand and nothing else
    // between it and the computer: a thread parked or stopped while its call
    // queued must not reach it.
    if !(state.alive)() {
        return ended();
    }
    let target = format!(
        "{}{}",
        state.origin,
        parts
            .uri
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str)
    );
    let mut headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        if !per_hop(name) {
            headers.append(name.clone(), value.clone());
        }
    }
    let sent = state
        .client
        .request(parts.method, target)
        .headers(headers)
        .bearer_auth(&state.token)
        .body(bytes)
        .send()
        .await;
    let upstream = match sent {
        Ok(upstream) => upstream,
        Err(error) => {
            return answer(
                StatusCode::BAD_GATEWAY,
                json!({ "error": format!("The computer did not answer: {error}") }),
            );
        }
    };
    let mut response = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers() {
        if !per_hop(name) {
            response = response.header(name, value);
        }
    }
    // The lease is held until the answer has been read to the end.
    let stream = upstream.bytes_stream().map(move |chunk| {
        let _holding = &call;
        chunk
    });
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| answer(StatusCode::BAD_GATEWAY, json!({ "error": "Bad answer." })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(key: &str) -> Holder {
        Holder::new(key, format!("the {key} thread"))
    }

    /// A call from a thread whose authority is current.
    async fn take(leases: &Arc<Leases>, persona: &str, by: &Holder) -> Result<Call, Refused> {
        leases.take(persona, by, &|| true).await
    }

    fn quick() -> Arc<Leases> {
        Leases::with(Timing {
            idle: Duration::from_millis(400),
            wait: Duration::from_millis(100),
        })
    }

    #[tokio::test]
    async fn one_thread_holds_the_computer_and_another_is_told_who_has_it() {
        let leases = quick();
        let (main, side) = (holder("main"), holder("side"));
        let first = take(&leases, "ada", &main).await.unwrap();
        let busy = take(&leases, "ada", &side)
            .await
            .err()
            .expect("it is held")
            .to_string();
        assert!(busy.contains("the main thread"), "{busy}");
        assert!(busy.contains("busy"));
        drop(first);
        // Between calls it is still the main thread's.
        assert!(take(&leases, "ada", &side).await.is_err());
        assert!(take(&leases, "ada", &main).await.is_ok());
    }

    #[tokio::test]
    async fn another_teammates_computer_is_its_own() {
        let leases = quick();
        let _ada = take(&leases, "ada", &holder("main")).await.unwrap();
        assert!(take(&leases, "bob", &holder("side")).await.is_ok());
    }

    #[tokio::test]
    async fn a_release_hands_it_to_the_thread_that_was_waiting() {
        let leases = Leases::with(Timing {
            idle: Duration::from_secs(60),
            wait: Duration::from_secs(5),
        });
        let (main, side) = (holder("main"), holder("side"));
        drop(take(&leases, "ada", &main).await.unwrap());
        let waiting = {
            let leases = leases.clone();
            let side = side.clone();
            tokio::spawn(async move { take(&leases, "ada", &side).await.map(|_| ()) })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "it waits while the other holds it");
        leases.release("ada", &main);
        waiting.await.unwrap().unwrap();
        assert_eq!(leases.holder("ada").as_deref(), Some("side"));
    }

    #[tokio::test]
    async fn a_thread_that_stops_using_it_lets_go_but_a_call_in_flight_is_never_taken() {
        let leases = quick();
        let (main, side) = (holder("main"), holder("side"));
        let call = take(&leases, "ada", &main).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            take(&leases, "ada", &side).await.is_err(),
            "a call that has not finished keeps it however long it takes"
        );
        drop(call);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(take(&leases, "ada", &side).await.is_ok());
    }

    #[tokio::test]
    async fn releasing_what_you_do_not_hold_changes_nothing() {
        let leases = quick();
        let _held = take(&leases, "ada", &holder("main")).await.unwrap();
        leases.release("ada", &holder("side"));
        assert_eq!(leases.holder("ada").as_deref(), Some("main"));
    }

    #[tokio::test]
    async fn a_retiring_agent_cannot_release_its_replacements_hold() {
        let leases = quick();
        let old = holder("dm");
        let replacement = holder("dm");
        assert_eq!(old.key, replacement.key, "the same thread, another agent");
        drop(take(&leases, "ada", &old).await.unwrap());
        leases.release("ada", &old);
        let _held = take(&leases, "ada", &replacement).await.unwrap();
        // The old agent's queue finishes, and its gate shuts down, late.
        leases.release("ada", &old);
        assert_eq!(leases.holder("ada").as_deref(), Some("dm"));
        assert!(
            take(&leases, "ada", &holder("side")).await.is_err(),
            "the replacement still has the computer"
        );
        // And the replacement is no more the old one's to call on.
        let call = take(&leases, "ada", &old).await;
        assert!(matches!(call, Err(Refused::Busy(_))));
    }

    #[tokio::test]
    async fn a_release_while_a_call_is_in_flight_keeps_the_computer_until_it_ends() {
        let leases = Leases::with(Timing {
            idle: Duration::from_secs(60),
            wait: Duration::from_millis(100),
        });
        let (main, side) = (holder("main"), holder("side"));
        let call = take(&leases, "ada", &main).await.unwrap();
        leases.release("ada", &main);
        assert!(
            take(&leases, "ada", &side).await.is_err(),
            "the upstream operation is still driving the computer"
        );
        assert_eq!(leases.holder("ada").as_deref(), Some("main"));
        drop(call);
        assert_eq!(leases.holder("ada"), None, "it is let go of when it ends");
        assert!(take(&leases, "ada", &side).await.is_ok());
    }

    #[tokio::test]
    async fn a_waiting_call_whose_authority_ends_is_cancelled_and_never_given_the_computer() {
        let leases = Leases::with(Timing {
            idle: Duration::from_secs(60),
            wait: Duration::from_secs(5),
        });
        let (main, side) = (holder("main"), holder("side"));
        let call = take(&leases, "ada", &main).await.unwrap();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let waiting = {
            let (leases, side, alive) = (leases.clone(), side.clone(), alive.clone());
            tokio::spawn(async move {
                leases
                    .take("ada", &side, &move || {
                        alive.load(std::sync::atomic::Ordering::SeqCst)
                    })
                    .await
                    .map(|_| ())
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        let refused = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("it does not wait out the lease")
            .unwrap();
        assert_eq!(refused, Err(Refused::Revoked));
        drop(call);
        leases.release("ada", &main);
        assert_eq!(leases.holder("ada"), None, "it was not handed the computer");
    }

    /// A computer that says only what it was asked, on a port of its own.
    async fn upstream() -> (super::super::Ready, Arc<Mutex<Vec<String>>>) {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let router = axum::Router::new().fallback(move |request: Request| {
            let record = record.clone();
            async move {
                let authorized = request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .is_some_and(|value| value == "Bearer real-token");
                let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                    .await
                    .unwrap();
                record
                    .lock()
                    .unwrap()
                    .push(format!("{authorized}:{}", String::from_utf8_lossy(&bytes)));
                answer(
                    StatusCode::OK,
                    json!({ "jsonrpc": "2.0", "id": 1, "result": "ok" }),
                )
            }
        });
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (
            super::super::Ready {
                url: format!("http://127.0.0.1:{port}/mcp"),
                token: "real-token".to_string(),
            },
            seen,
        )
    }

    async fn post(gate: &Gate, body: Value) -> (StatusCode, Value) {
        let response = reqwest::Client::new()
            .post(&gate.url)
            .bearer_auth(&gate.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn the_gate_forwards_with_the_computers_own_bearer_and_serializes_calls() {
        let (computer, seen) = upstream().await;
        // Idle far beyond the test, so a slow machine cannot expire the hold.
        let leases = Leases::with(Timing {
            idle: Duration::from_secs(30),
            wait: Duration::from_millis(100),
        });
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let gate = |key: &'static str| {
            let leases = leases.clone();
            let computer = computer.clone();
            let alive = alive.clone();
            async move {
                serve(&computer, leases, "ada", holder(key), move || {
                    alive.load(std::sync::atomic::Ordering::SeqCst)
                })
                .await
                .unwrap()
            }
        };
        let main = gate("main").await;
        let side = gate("side").await;
        let call = json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"click"}});

        let (status, body) =
            post(&main, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
        assert_eq!(
            (status, body["result"].as_str()),
            (StatusCode::OK, Some("ok"))
        );
        let (_, body) = post(&main, call.clone()).await;
        assert_eq!(body["result"], "ok");
        assert!(
            lock_seen(&seen)
                .iter()
                .all(|line| line.starts_with("true:")),
            "the computer sees only its own token, never the gate's"
        );

        let (_, body) = post(&side, call.clone()).await;
        assert_eq!(body["id"], 7, "the refusal answers the call that was made");
        assert_eq!(body["result"]["isError"], true);
        assert!(
            body["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("the main thread")
        );
        let (_, listed) = post(&side, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
        assert_eq!(
            listed["result"], "ok",
            "listing tools is not driving the computer"
        );

        let (status, _) = reqwest::Client::new()
            .post(&main.url)
            .json(&call)
            .send()
            .await
            .map(|response| (response.status(), ()))
            .unwrap();
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the gate answers only its agent"
        );

        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(
            leases.holder("ada"),
            None,
            "a gate that ends releases what it held"
        );
    }

    #[tokio::test]
    async fn a_call_queued_at_the_gate_when_its_thread_is_revoked_never_reaches_the_computer() {
        let (computer, seen) = upstream().await;
        let leases = Leases::with(Timing {
            idle: Duration::from_secs(60),
            wait: Duration::from_secs(5),
        });
        let (main, side) = (holder("main"), holder("side"));
        let on = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let gate = {
            let (computer, leases, on) = (computer.clone(), leases.clone(), on.clone());
            move |by: Holder, own: bool| {
                let (computer, leases, on) = (computer.clone(), leases.clone(), on.clone());
                async move {
                    serve(&computer, leases, "ada", by, move || {
                        !own || on.load(std::sync::atomic::Ordering::SeqCst)
                    })
                    .await
                    .unwrap()
                }
            }
        };
        let main_gate = gate(main.clone(), false).await;
        let side_gate = gate(side.clone(), true).await;
        let call = json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"click"}});
        post(&main_gate, call.clone()).await;
        let before = lock_seen(&seen).len();

        let queued = {
            let call = call.clone();
            tokio::spawn(async move { post(&side_gate, call).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!queued.is_finished(), "the call waits for the computer");
        // The thread is parked while its call waits, and then the computer
        // comes free.
        on.store(false, std::sync::atomic::Ordering::SeqCst);
        leases.release("ada", &main);
        let (status, _) = tokio::time::timeout(Duration::from_secs(3), queued)
            .await
            .expect("a revoked call does not wait out the lease")
            .unwrap();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            lock_seen(&seen).len(),
            before,
            "nothing a revoked thread queued was sent to the computer"
        );
    }

    fn lock_seen(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        seen.lock().unwrap().clone()
    }
}
