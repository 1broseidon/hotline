//! The agreed computer door, with a clock the desk's acceptance tests control.
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::time::{Duration, Instant};

#[derive(Default)]
struct StateData {
    secrets: BTreeMap<String, Value>,
    uses: Vec<Value>,
    answers: Vec<Value>,
    allows: BTreeMap<(String, String), Instant>,
    #[cfg(unix)]
    next: u64,
    polls: usize,
}

#[derive(Clone, Default)]
pub(crate) struct Door(Arc<Mutex<StateData>>);

impl Door {
    pub(crate) fn replace(&self, secrets: &BTreeMap<String, Value>) {
        self.0.lock().unwrap().secrets.clone_from(secrets);
    }
    #[cfg(unix)]
    pub(crate) fn hold(&self, secret: &str, site: &str) -> Option<String> {
        let mut state = self.0.lock().unwrap();
        let value = state.secrets[secret].clone();
        if value["use"] != "ask"
            || state
                .allows
                .get(&(secret.into(), site.into()))
                .is_some_and(|until| *until > Instant::now())
        {
            return None;
        }
        state.next += 1;
        let id = format!("use-{}", state.next);
        let now = crate::session::now_ms();
        state.uses.push(json!({"id": id, "secret":secret, "kind":value["kind"], "site":site, "path":"/login", "askedAt":now,"expiresAt":now + 300_000}));
        Some(id)
    }
    #[cfg(unix)]
    pub(crate) fn expire(&self, id: &str) {
        self.0.lock().unwrap().uses.retain(|u| u["id"] != id);
    }
    #[cfg(unix)]
    pub(crate) fn answers(&self) -> Vec<Value> {
        self.0.lock().unwrap().answers.clone()
    }
    #[cfg(unix)]
    pub(crate) fn polls(&self) -> usize {
        self.0.lock().unwrap().polls
    }
}

fn authenticated(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h.starts_with("Bearer ") && h.len() > 7)
}

pub(crate) async fn held(
    State(taken): State<crate::computer::guide::fake::Taken>,
    headers: HeaderMap,
) -> (StatusCode, String) {
    if !authenticated(&headers) {
        return (StatusCode::UNAUTHORIZED, encoded(json!({})));
    }
    let mut state = taken.uses.0.lock().unwrap();
    state.polls += 1;
    (StatusCode::OK, encoded(json!({"uses": state.uses})))
}

pub(crate) async fn answer(
    State(taken): State<crate::computer::guide::fake::Taken>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    if !authenticated(&headers) {
        return (StatusCode::UNAUTHORIZED, encoded(json!({})));
    }
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !matches!(body["answer"].as_str(), Some("approve" | "allow" | "deny"))
        || !body["id"].is_string()
    {
        return (StatusCode::BAD_REQUEST, encoded(json!({})));
    }
    let mut state = taken.uses.0.lock().unwrap();
    let Some(index) = state.uses.iter().position(|u| u["id"] == body["id"]) else {
        return (StatusCode::CONFLICT, encoded(json!({})));
    };
    let held = state.uses.remove(index);
    if body["answer"] == "allow" {
        state.allows.insert(
            (
                held["secret"].as_str().unwrap().into(),
                held["site"].as_str().unwrap().into(),
            ),
            Instant::now() + Duration::from_secs(600),
        );
    }
    let reply = if body["answer"] == "deny" {
        "denied"
    } else {
        "approved"
    };
    state.answers.push(body);
    (StatusCode::OK, encoded(json!({"state":reply})))
}

fn encoded(value: Value) -> String {
    value.to_string()
}
