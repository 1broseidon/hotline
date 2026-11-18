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
//! Not every call drives the desktop. Reading what the computer is and has,
//! and starting and reading managed shell jobs, the computer itself runs
//! beside whoever is driving, so those calls ([`shares`]) go through without
//! the lease, whoever holds it: they neither take it, keep it, nor let go of
//! it. Only calls named on that list, one call to a request, are let through
//! this way; anything else, a tool or an action added later among them, needs
//! the lease as before. A call let through still counts as the computer
//! being in use ([`Leases::in_use`]), so the sweep does not stop a computer a
//! thread is building on.
//!
//! The gate reaches only the computer it was made for, with the bearer the
//! teammate's grant already carried: it adds no authority, and a thread whose
//! lease has been revoked is refused at the gate before anything is forwarded,
//! including a call that was waiting for the computer when it was revoked,
//! and one that needs no lease.

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
    /// Calls let through without the lease ([`shares`]), kept apart from the
    /// lease so they never touch it: how many are in flight for each
    /// teammate's computer, and when the last one ended.
    shared: Mutex<HashMap<String, SharedUse>>,
    freed: Notify,
    timing: Timing,
}

#[derive(Default)]
struct SharedUse {
    in_flight: usize,
    last: Option<Instant>,
}

/// How long after a call let through without the lease the computer still
/// counts as in use: longer than the minute between sweeps, so each such
/// call is seen by one.
const SHARED_USE_COUNTS_FOR: Duration = Duration::from_secs(150);

/// A call let through without the lease. Dropping it ends the call.
pub(crate) struct SharedCall {
    leases: Arc<Leases>,
    persona_id: String,
}

impl Drop for SharedCall {
    fn drop(&mut self) {
        let mut shared = self
            .leases
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = shared.get_mut(&self.persona_id) {
            entry.in_flight = entry.in_flight.saturating_sub(1);
            entry.last = Some(Instant::now());
        }
    }
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
            shared: Mutex::new(HashMap::new()),
            freed: Notify::new(),
            timing,
        })
    }

    /// A call that needs no lease ([`shares`]) begins. Nothing of the lease
    /// is touched: not who holds it, not its holder's calls in flight, not
    /// its idle clock. It is only counted as use of the computer.
    pub(crate) fn share(self: &Arc<Self>, persona_id: &str) -> SharedCall {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(persona_id.to_string())
            .or_default()
            .in_flight += 1;
        SharedCall {
            leases: self.clone(),
            persona_id: persona_id.to_string(),
        }
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
                    "The computer is busy: {title} is using it right now. {WHILE_ANOTHER_DRIVES} Work on something else, or try this again in a minute."
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

    /// The teammates whose computer is in use now: someone holds it, in a
    /// call or between calls of a turn that has not let go, or a call that
    /// needs no lease is in flight or ended within [`SHARED_USE_COUNTS_FOR`].
    /// A sweep leaves these running.
    pub(crate) fn in_use(&self) -> std::collections::HashSet<String> {
        let mut ids: std::collections::HashSet<String> = self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared.retain(|_, entry| {
            entry.in_flight > 0
                || entry
                    .last
                    .is_some_and(|last| last.elapsed() < SHARED_USE_COUNTS_FOR)
        });
        ids.extend(shared.keys().cloned());
        ids
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

/// What a thread can still do while another drives, as a thread is told it
/// ([`Refused::Busy`], and the thread briefs that name it).
pub(crate) const WHILE_ANOTHER_DRIVES: &str = "While another thread drives, these work without waiting: state info, guide, catalog, manifest, login_list and snapshot_list; shell exec, start, list, status, read and wait; files get and list. Everything else, including shell cancel and write, waits for the computer, and a job started meanwhile must not open windows or drive the desktop.";

/// The calls that go through without the lease: each a tool and one of its
/// actions, and nothing else. The computer runs each of these beside a call
/// that drives the desktop, on every release the desk talks to (0.11.0 on):
///
/// - `state` `info`, `guide`, `catalog`, `manifest`, `login_list` and
///   `snapshot_list` read what the computer is and has. None takes the
///   computer's own hold on the desktop or changes anything.
/// - `shell` `exec` and `start` start a managed job, which runs without
///   holding the desktop. The computer still admits it behind a desktop call
///   in flight and refuses it while the person holds the screen, as for any
///   caller. `launch` is the same start on the computer, but it is what an
///   app with a window is opened with, so it is left to the holder; so is
///   `run`, whose desktop runs open a window and wait on it.
/// - `shell` `list`, `status`, `read` and `wait` read jobs and wait on one.
///   Cancelling a job or writing to its input acts on a job the gate cannot
///   tell is this thread's, as every thread is one caller to the computer,
///   so those need the lease.
/// - `files` `get` and `list` read under the computer's home.
///
/// A shell call that names no action is an `exec`, as the computer reads it.
fn shares(tool: &str, action: &str) -> bool {
    matches!(
        (tool, action),
        (
            "state",
            "info" | "guide" | "catalog" | "manifest" | "login_list" | "snapshot_list"
        ) | (
            "shell",
            "exec" | "start" | "list" | "status" | "read" | "wait"
        ) | ("files", "get" | "list")
    )
}

/// What a request is, as far as the lease goes.
#[derive(Debug, PartialEq)]
enum Asks {
    /// No tool is called: the lease is not wanted.
    Nothing,
    /// One call on [`shares`]: it goes through without the lease.
    Shared(Value),
    /// Any other call, or a batch with one in it, or a call the gate cannot
    /// read for certain: it needs the lease. The id is what a refusal
    /// answers.
    Lease(Value),
}

/// Reads a posted request for whether it calls a tool, and which. Only a
/// request that is one JSON-RPC message, with no key given twice anywhere in
/// it, a tool name and an action that are strings (a shell call may leave its
/// action out), is ever [`Asks::Shared`]; whatever else calls a tool needs the
/// lease, and so does a body the gate cannot read as JSON at all, since what
/// the gate cannot read it cannot say calls nothing.
fn asks(body: &[u8]) -> Asks {
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return Asks::Lease(Value::Null);
    };
    let calls = |one: &Value| one.get("method").and_then(Value::as_str) == Some("tools/call");
    let one = match &message {
        Value::Array(many) if many.iter().any(calls) => return Asks::Lease(Value::Null),
        one if calls(one) => one,
        _ => return Asks::Nothing,
    };
    let id = one.get("id").cloned().unwrap_or(Value::Null);
    // Two parsers that disagree about a key given twice would let the gate
    // read one call and the computer run another.
    if !unique_keys(body) {
        return Asks::Lease(id);
    }
    let params = one.get("params");
    let tool = params
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str);
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .and_then(Value::as_object);
    let action = arguments.and_then(|arguments| match arguments.get("action") {
        Some(Value::String(action)) => Some(action.as_str()),
        None if tool == Some("shell") => Some("exec"),
        _ => None,
    });
    match (tool, action) {
        (Some(tool), Some(action)) if shares(tool, action) => Asks::Shared(id),
        _ => Asks::Lease(id),
    }
}

/// Whether no object anywhere in `body` gives a key twice.
fn unique_keys(body: &[u8]) -> bool {
    use serde::de::{DeserializeSeed, Deserializer, Error, MapAccess, SeqAccess, Visitor};

    struct Unique;

    impl<'de> DeserializeSeed<'de> for Unique {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
            deserializer.deserialize_any(self)
        }
    }

    impl<'de> Visitor<'de> for Unique {
        type Value = ();
        fn expecting(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            out.write_str("JSON")
        }
        fn visit_bool<E>(self, _: bool) -> Result<(), E> {
            Ok(())
        }
        fn visit_i64<E>(self, _: i64) -> Result<(), E> {
            Ok(())
        }
        fn visit_u64<E>(self, _: u64) -> Result<(), E> {
            Ok(())
        }
        fn visit_f64<E>(self, _: f64) -> Result<(), E> {
            Ok(())
        }
        fn visit_str<E>(self, _: &str) -> Result<(), E> {
            Ok(())
        }
        fn visit_unit<E>(self) -> Result<(), E> {
            Ok(())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            while seq.next_element_seed(Unique)?.is_some() {}
            Ok(())
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            let mut seen = std::collections::HashSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !seen.insert(key) {
                    return Err(A::Error::custom("a key given twice"));
                }
                map.next_value_seed(Unique)?;
            }
            Ok(())
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(body);
    Unique.deserialize(&mut deserializer).is_ok() && deserializer.end().is_ok()
}

fn answer(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("a json answer is a response")
}

/// What a call to the computer holds while it is answered.
enum Hold {
    Lease { _call: Call },
    Shared { _call: SharedCall },
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
        match asks(&bytes) {
            // Taken here, after the authority was checked above and before
            // it is checked again below, like a call with the lease.
            Asks::Shared(_) => Some(Hold::Shared {
                _call: state.leases.share(&state.persona_id),
            }),
            Asks::Lease(id) => match state
                .leases
                .take(&state.persona_id, &state.holder, &*state.alive)
                .await
            {
                Ok(call) => Some(Hold::Lease { _call: call }),
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
            Asks::Nothing => None,
        }
    } else {
        None
    };
    // The authority is checked again with the lease in hand, or with a call
    // that needs none, and nothing else between it and the computer: a
    // thread parked or stopped while its call queued must not reach it.
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
    // The lease, or the count of a call that needs none, is held until the
    // answer has been read to the end.
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
    use crate::test_support::until;

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

        let (status, ()) = reqwest::Client::new()
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
        until("a gate that ends releases what it held", || {
            leases.holder("ada").is_none()
        })
        .await;
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

    fn tool_call(name: &str, arguments: Value) -> Vec<u8> {
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":name,"arguments":arguments}})
            .to_string()
            .into_bytes()
    }

    #[test]
    fn only_the_named_reads_and_jobs_go_without_the_lease() {
        let shared = [
            ("state", "info"),
            ("state", "guide"),
            ("state", "catalog"),
            ("state", "manifest"),
            ("state", "login_list"),
            ("state", "snapshot_list"),
            ("shell", "exec"),
            ("shell", "start"),
            ("shell", "list"),
            ("shell", "status"),
            ("shell", "read"),
            ("shell", "wait"),
            ("files", "get"),
            ("files", "list"),
        ];
        for (tool, action) in shared {
            assert_eq!(
                asks(&tool_call(tool, json!({"action": action}))),
                Asks::Shared(json!(9)),
                "{tool} {action}"
            );
        }
        assert_eq!(
            asks(&tool_call("shell", json!({"command": "ls"}))),
            Asks::Shared(json!(9)),
            "a shell call with no action is an exec"
        );
        let leased = [
            ("input", "click"),
            ("input", "clipboard_read"),
            ("capture", "tree"),
            ("windows", "list"),
            ("windows", "close"),
            ("wait", "text"),
            ("browser", "text"),
            ("browser", "fill"),
            ("shell", "launch"),
            ("shell", "run"),
            ("shell", "ready"),
            ("shell", "show"),
            ("shell", "cancel"),
            ("shell", "write"),
            ("state", "prepare"),
            ("state", "control"),
            ("state", "release"),
            ("state", "login_save"),
            ("state", "login_load"),
            ("state", "login_delete"),
            ("state", "snapshot_save"),
            ("state", "snapshot_load"),
            ("state", "snapshot_delete"),
            ("files", "put"),
            ("files", "download"),
            ("files", "extract"),
            ("files", "run"),
            // Tools and actions the computer may add later, and near misses.
            ("state", "info2"),
            ("shell", "List"),
            ("Shell", "list"),
            ("shell", " list"),
            ("notes", "list"),
            ("", ""),
        ];
        for (tool, action) in leased {
            assert_eq!(
                asks(&tool_call(tool, json!({"action": action}))),
                Asks::Lease(json!(9)),
                "{tool} {action}"
            );
        }
        for arguments in [
            json!({}),
            json!(null),
            json!(["list"]),
            json!({"action": 1}),
            json!({"action": ["list"]}),
            json!({"action": null}),
        ] {
            assert_eq!(
                asks(&tool_call("state", arguments.clone())),
                Asks::Lease(json!(9)),
                "{arguments}"
            );
        }
        assert_eq!(
            asks(&tool_call("shell", json!({"action": null}))),
            Asks::Lease(json!(9)),
            "an action that is there is a string, or the call needs the lease"
        );
        assert_eq!(
            asks(&tool_call("files", json!({"path": "a"}))),
            Asks::Lease(json!(9)),
            "only shell's action has a default"
        );
        assert_eq!(
            asks(
                br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"arguments":{"action":"list"}}}"#
            ),
            Asks::Lease(json!(3)),
            "a call that names no tool"
        );
    }

    #[test]
    fn a_call_the_gate_cannot_read_for_certain_needs_the_lease() {
        let cases: [&[u8]; 7] = [
            // A batch, even of calls that would each go through.
            br#"[{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"action":"list"}}}]"#,
            // A key given twice, which another parser might read the other way.
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"input","name":"shell","arguments":{"action":"list"}}}"#,
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"action":"cancel","action":"list"}}}"#,
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","name":"shell","arguments":{"action":"list"}}}"#,
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"action":"list","job_id":"a","job_id":"b"}}}"#,
            // Not JSON, or not only JSON.
            b"\x1f\x8b\x08\x00 compressed",
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell","arguments":{"action":"list"}}} {}"#,
        ];
        for body in cases {
            assert!(
                matches!(asks(body), Asks::Lease(_)),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert_eq!(
            asks(br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#),
            Asks::Nothing
        );
        assert_eq!(
            asks(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Asks::Nothing
        );
        assert!(unique_keys(br#"{"a":{"b":[{"c":1},{"c":2}]},"b":"a"}"#));
    }

    #[tokio::test]
    async fn a_call_without_the_lease_leaves_the_holders_hold_as_it_was() {
        let leases = quick();
        let (main, side) = (holder("main"), holder("side"));
        // Nobody holds it, and a call that needs no lease does not take it.
        drop(leases.share("ada"));
        assert_eq!(leases.holder("ada"), None);
        assert!(leases.in_use().contains("ada"), "it was used");
        assert!(!leases.in_use().contains("bob"));

        // The holder's call in flight, let go of meanwhile, still ends the
        // hold when it ends, whatever was let through beside it.
        let call = take(&leases, "ada", &main).await.unwrap();
        leases.release("ada", &main);
        let beside = leases.share("ada");
        assert_eq!(leases.holder("ada").as_deref(), Some("main"));
        drop(beside);
        assert_eq!(leases.holder("ada").as_deref(), Some("main"));
        drop(call);
        assert_eq!(leases.holder("ada"), None);

        // Nor does one keep an idle holder's hold alive.
        drop(take(&leases, "ada", &main).await.unwrap());
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(500) {
            drop(leases.share("ada"));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(take(&leases, "ada", &side).await.is_ok());
        assert_eq!(leases.holder("ada").as_deref(), Some("side"));
    }

    #[tokio::test]
    async fn another_thread_reads_and_runs_jobs_while_one_drives_but_drives_nothing() {
        let (computer, seen) = upstream().await;
        let leases = Leases::with(Timing {
            idle: Duration::from_secs(30),
            wait: Duration::from_millis(100),
        });
        let gate = |key: &'static str| {
            let (leases, computer) = (leases.clone(), computer.clone());
            async move {
                serve(&computer, leases, "ada", holder(key), || true)
                    .await
                    .unwrap()
            }
        };
        let main = gate("main").await;
        let side = gate("side").await;
        let call = |name: &str, action: &str| json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":{"action":action}}});
        let (_, body) = post(&main, call("input", "click")).await;
        assert_eq!(body["result"], "ok");

        for (tool, action) in [("shell", "list"), ("shell", "start"), ("state", "info")] {
            let before = lock_seen(&seen).len();
            let (_, body) = post(&side, call(tool, action)).await;
            assert_eq!(body["result"], "ok", "{tool} {action}");
            let seen = lock_seen(&seen);
            assert_eq!(seen.len(), before + 1);
            assert!(
                seen[before].starts_with("true:"),
                "forwarded with the computer's own bearer"
            );
            assert_eq!(leases.holder("ada").as_deref(), Some("main"));
        }
        for (tool, action) in [
            ("input", "click"),
            ("shell", "cancel"),
            ("shell", "write"),
            ("state", "prepare"),
        ] {
            let before = lock_seen(&seen).len();
            let (_, body) = post(&side, call(tool, action)).await;
            assert_eq!(body["result"]["isError"], true, "{tool} {action}");
            let text = body["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("the main thread"), "{text}");
            assert!(text.contains("shell exec, start"), "{text}");
            assert_eq!(lock_seen(&seen).len(), before, "nothing was forwarded");
        }
        assert_eq!(leases.holder("ada").as_deref(), Some("main"));
    }

    /// A gate's state as `serve` makes it, for calling `forward` directly.
    fn shared_state(
        computer: &super::super::Ready,
        leases: &Arc<Leases>,
        alive: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Arc<Shared> {
        Arc::new(Shared {
            client: reqwest::Client::new(),
            origin: url::Url::parse(&computer.url)
                .unwrap()
                .origin()
                .ascii_serialization(),
            token: computer.token.clone(),
            leases: leases.clone(),
            persona_id: "ada".into(),
            holder: holder("side"),
            alive: Arc::new(alive),
        })
    }

    fn request(body: Vec<u8>) -> Request {
        axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn a_revoked_thread_is_refused_even_a_call_that_needs_no_lease() {
        let (computer, seen) = upstream().await;
        let leases = quick();
        let list = || tool_call("shell", json!({"action": "list"}));

        let revoked = shared_state(&computer, &leases, || false);
        let refused = forward(State(revoked), request(list())).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        // Revoked between the first look and the forward.
        let looks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = looks.clone();
        let revoked_on_the_way = shared_state(&computer, &leases, move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        });
        let refused = forward(State(revoked_on_the_way), request(list())).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(looks.load(std::sync::atomic::Ordering::SeqCst), 2);

        assert!(lock_seen(&seen).is_empty(), "nothing reached the computer");
        assert_eq!(leases.holder("ada"), None);

        let current = shared_state(&computer, &leases, || true);
        let answered = forward(State(current), request(list())).await;
        assert_eq!(answered.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(answered.into_body(), 1 << 20).await;
        assert_eq!(lock_seen(&seen).len(), 1);
        assert_eq!(leases.holder("ada"), None, "and it took no lease");
    }
}
