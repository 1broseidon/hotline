//! Handing a running computer the secrets its teammate is granted, out of
//! band.
//!
//! The computer keeps the set in memory and puts each one in the environment
//! of every job the agent starts; it never answers a value back, over any
//! route, and redacts them from what its tools return. The desk replaces the
//! whole set at every grant — session start, reattach, a stored value
//! changing — so a rotation or a revocation reaches a running machine
//! without a restart. The values ride desk → container over the container's
//! authenticated loopback port, bearer in a header, and touch no tape, model
//! or log on the way.

use super::Ready;
use std::collections::BTreeMap;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Why a computer did not take the set.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The release has no `/secrets` route: it predates them. The pane's
    /// Update is the fix.
    TooOld,
    /// It could not be reached, or refused for a reason of its own.
    Failed(String),
}

/// Replaces the computer's whole set with `secrets`, through `PUT /secrets`.
/// An empty map clears it, which is how a revocation lands.
pub async fn deliver(ready: &Ready, secrets: &BTreeMap<String, String>) -> Result<(), Refusal> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Refusal::Failed(error.to_string()))?;
    let response = client
        .put(format!("{base}/secrets"))
        .bearer_auth(&ready.token)
        .json(secrets)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Refusal::TooOld);
    }
    let reason = response.text().await.unwrap_or_default();
    let reason = reason.trim();
    Err(Refusal::Failed(if reason.is_empty() {
        format!("The computer refused them ({status}).")
    } else {
        format!("The computer refused them ({status}): {reason}")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::put;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Fake {
        /// Every body that arrived with the right bearer, in order.
        taken: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
    }

    async fn secrets(State(fake): State<Fake>, headers: HeaderMap, body: String) -> StatusCode {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if bearer != Some("the-token") {
            return StatusCode::UNAUTHORIZED;
        }
        let Ok(set) = serde_json::from_str::<BTreeMap<String, String>>(&body) else {
            return StatusCode::BAD_REQUEST;
        };
        fake.taken.lock().unwrap().push(set);
        StatusCode::NO_CONTENT
    }

    /// A stand-in computer; `with_route` false is a release from before
    /// secrets, which knows no such path.
    async fn computer(with_route: bool) -> (Ready, Fake, tokio::task::JoinHandle<()>) {
        let fake = Fake::default();
        let mut app = axum::Router::new();
        if with_route {
            app = app.route("/secrets", put(secrets));
        }
        let app = app.with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let serving = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "the-token".to_owned(),
        };
        (ready, fake, serving)
    }

    #[tokio::test]
    async fn the_set_is_put_whole_with_the_bearer_in_a_header() {
        let (ready, fake, serving) = computer(true).await;
        let mut set = BTreeMap::new();
        set.insert(
            "GITHUB_TOKEN".to_owned(),
            "ghp_notarealtoken0001".to_owned(),
        );
        set.insert("NPM_TOKEN".to_owned(), "npm_notarealtoken0002".to_owned());
        deliver(&ready, &set).await.expect("taken");
        deliver(&ready, &BTreeMap::new())
            .await
            .expect("an empty set clears");
        let taken = fake.taken.lock().unwrap().clone();
        assert_eq!(taken, vec![set, BTreeMap::new()]);
        serving.abort();
    }

    #[tokio::test]
    async fn a_release_from_before_secrets_is_told_apart_from_a_failure() {
        let (ready, _, serving) = computer(false).await;
        assert_eq!(
            deliver(&ready, &BTreeMap::new()).await,
            Err(Refusal::TooOld)
        );
        serving.abort();

        let (ready, fake, serving) = computer(true).await;
        let wrong = Ready {
            token: "not-the-token".to_owned(),
            ..ready
        };
        let refused = deliver(&wrong, &BTreeMap::new()).await.unwrap_err();
        assert!(
            matches!(&refused, Refusal::Failed(reason) if reason.contains("401")),
            "{refused:?}"
        );
        assert!(fake.taken.lock().unwrap().is_empty());
        serving.abort();
    }
}
