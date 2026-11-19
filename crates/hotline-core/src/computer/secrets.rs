//! Handing a running computer the secrets its teammate is granted, out of
//! band.
//!
//! The computer keeps the set in memory and puts each one where its kind
//! says — a variable in the environment of every job the agent starts, a
//! login behind `browser fill` on its own sites, a passkey in the browser's
//! authenticator; it never answers a value back, over any route, and
//! redacts them from what its tools return. The desk replaces the whole set
//! at every grant — session start, reattach, a stored value changing — so a
//! rotation or a revocation reaches a running machine without a restart.
//! The values ride desk → container over the container's authenticated
//! loopback port, bearer in a header, and touch no tape, model or log on
//! the way.
//!
//! What a computer is handed depends on what it proves it is. Each
//! handing over first asks `GET /capabilities` with a fresh nonce. A
//! computer that runs split — its jobs a user apart from the broker that
//! holds values — answers an `attest` only the holder of this computer's
//! bearer can make, and takes every grant through the v2 form: readable
//! variables, keys with their "Used with", logins and passkeys. One that
//! has no such door, or runs as one user (legacy), is handed what 0.49
//! handed it — readable variables, logins and passkeys, in the v1 form
//! every release takes — and a key granted protected waits on the desk
//! until it is updated, since its programs could read it there. One whose
//! `attest` is wrong is handed nothing at all: what answers the port is not
//! this computer.

use super::Ready;
use crate::vault::StoredSecret;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// The v2 form's media type; a body without it is read as v1.
const V2: &str = "application/vnd.hotline.secrets.v2+json";

/// What the `attest` is the HMAC of, after the boot and the nonce are
/// joined on: the broker's own words, so it is never another door's MAC.
const ATTEST_CONTEXT: &str = "hotline-broker-v1";

/// Why a computer did not take the set.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The release has no `/secrets` route: it predates them. The pane's
    /// Update is the fix.
    TooOld,
    /// It could not be reached, or refused for a reason of its own.
    Failed(String),
}

/// What a computer proved about keeping secrets from its jobs.
#[derive(Debug, PartialEq, Eq)]
pub enum Door {
    /// Split, and its broker holds this computer's bearer: it answered for
    /// the boot it is on.
    Protected,
    /// No `/capabilities`, a release before 0.13, or one that says it runs
    /// legacy: today's one user, which a job shares with every value.
    Unprotected,
}

/// Asks the computer what it is, with a nonce of this asking's own. An
/// answer that says protected but whose `attest` this bearer did not make
/// is refused, so nothing is sent to whatever made it.
pub async fn door(ready: &Ready) -> Result<Door, Refusal> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let client = super::door_client()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Refusal::Failed(error.to_string()))?;
    let response = client
        .get(format!("{base}/capabilities?nonce={nonce}"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    match response.status() {
        reqwest::StatusCode::NOT_FOUND => return Ok(Door::Unprotected),
        status if !status.is_success() => {
            return Err(Refusal::Failed(format!(
                "The computer would not say what it is ({status})."
            )));
        }
        _ => {}
    }
    let answer: Value = response
        .json()
        .await
        .map_err(|_| Refusal::Failed("The computer's capabilities could not be read.".into()))?;
    let split = answer["protected"]["v"] == 1 && answer["protected"]["mode"] == "split";
    if !split {
        return Ok(Door::Unprotected);
    }
    let boot = answer["boot"].as_str().unwrap_or_default();
    let attested = answer["attest"]
        .as_str()
        .and_then(|attest| {
            base64::engine::general_purpose::STANDARD
                .decode(attest)
                .ok()
        })
        .is_some_and(|attest: Vec<u8>| {
            let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(ready.token.as_bytes()) else {
                return false;
            };
            mac.update(format!("{ATTEST_CONTEXT}|{boot}|{nonce}").as_bytes());
            mac.verify_slice(&attest).is_ok()
        });
    if boot.is_empty() || !attested {
        return Err(Refusal::Failed(
            "The computer did not prove it is this teammate's, so it was handed nothing.".into(),
        ));
    }
    Ok(Door::Protected)
}

/// One granted secret as the desk hands it: the record, and whether the
/// teammate's programs may read it.
#[derive(Clone, Debug)]
pub struct Granted {
    pub secret: StoredSecret,
    pub readable: bool,
}

/// What a handing over came to: whether the computer was protected, and
/// the names it was not handed: keys granted protected, which wait on an
/// unprotected computer for its update, and on a protected one for a "Used
/// with" to protect them by.
#[derive(Debug, PartialEq, Eq)]
pub struct Handed {
    pub protected: bool,
    pub waiting: Vec<String>,
}

/// The v1 set: a variable as its bare value, which every computer release
/// with a `/secrets` route takes; a login or a passkey as the record, kind
/// and all, which a release from 0.8 on takes.
pub fn wire_form(secrets: &BTreeMap<String, StoredSecret>) -> BTreeMap<String, Value> {
    secrets
        .iter()
        .map(|(name, secret)| {
            let value = match secret {
                StoredSecret::Variable { value, .. } => Value::String(value.clone()),
                typed => serde_json::to_value(typed).unwrap_or(Value::Null),
            };
            (name.clone(), value)
        })
        .collect()
}

/// One entry of the v2 set, or none for a key granted protected that has
/// no "Used with". A key's stand-in keeps its known prefix and its length,
/// so a program that checks the token's shape still runs.
fn entry_v2(granted: &Granted) -> Option<Value> {
    match &granted.secret {
        StoredSecret::Variable { value, .. } if granted.readable => {
            Some(json!({"kind": "variable", "value": value, "readable": true}))
        }
        StoredSecret::Variable {
            value,
            r#use: Some(usage),
        } => {
            let mut usage = serde_json::to_value(usage).ok()?;
            usage["standIn"] = json!({
                "prefix": crate::vault::stand_in_prefix(value),
                "length": value.chars().count(),
            });
            Some(json!({"kind": "variable", "value": value, "use": usage}))
        }
        StoredSecret::Variable { r#use: None, .. } => None,
        typed => serde_json::to_value(typed).ok(),
    }
}

/// Each v2 set's generation is past every one this desk sent before, across
/// a restart too, so a computer can tell the newest set and drop what an
/// older one gave its jobs.
fn next_generation() -> i64 {
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = chrono::Utc::now().timestamp_millis();
    let previous = LAST.fetch_max(now, Ordering::SeqCst);
    if previous < now {
        return now;
    }
    LAST.fetch_add(1, Ordering::SeqCst) + 1
}

/// Replaces the computer's whole set with what it can be handed of `set`,
/// through `PUT /secrets`. An empty set clears it, which is how a
/// revocation lands.
pub async fn deliver(ready: &Ready, set: &BTreeMap<String, Granted>) -> Result<Handed, Refusal> {
    let protected = door(ready).await? == Door::Protected;
    let mut waiting = Vec::new();
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let client = super::door_client()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Refusal::Failed(error.to_string()))?;
    let request = if protected {
        let mut secrets = serde_json::Map::new();
        for (name, granted) in set {
            match entry_v2(granted) {
                Some(entry) => {
                    secrets.insert(name.clone(), entry);
                }
                None => waiting.push(name.clone()),
            }
        }
        let body = json!({"v": 2, "generation": next_generation(), "secrets": secrets});
        client
            .put(format!("{base}/secrets"))
            .header(reqwest::header::CONTENT_TYPE, V2)
            .body(body.to_string())
    } else {
        let mut today = BTreeMap::new();
        for (name, granted) in set {
            match &granted.secret {
                StoredSecret::Variable { .. } if !granted.readable => waiting.push(name.clone()),
                secret => {
                    today.insert(name.clone(), secret.clone());
                }
            }
        }
        client
            .put(format!("{base}/secrets"))
            .json(&wire_form(&today))
    };
    let response = request
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    let status = response.status();
    if status.is_success() {
        return Ok(Handed { protected, waiting });
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Refusal::TooOld);
    }
    // An upstream error may echo the submitted body. Its status is enough
    // to report failure without copying a password into the desk's logs.
    Err(Refusal::Failed(format!(
        "The computer refused them ({status})."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::{get, put};
    use std::sync::{Arc, Mutex};

    /// What the stand-in computer says it is at `/capabilities`.
    #[derive(Clone, Copy, PartialEq)]
    enum Is {
        /// A release before 0.13: no such door.
        Old,
        /// 0.13 booted as one user.
        Legacy,
        /// 0.13 split, attesting with the bearer.
        Split,
        /// Says split, attests with another key.
        Forged,
    }

    /// A body as it arrived, with its content type.
    type Taken = (Option<String>, Value);

    #[derive(Clone)]
    struct Fake {
        is: Is,
        /// Every body that arrived with the right bearer, and its type.
        taken: Arc<Mutex<Vec<Taken>>>,
    }

    fn bearer(headers: &HeaderMap) -> Option<&str> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
    }

    async fn secrets(State(fake): State<Fake>, headers: HeaderMap, body: String) -> StatusCode {
        if bearer(&headers) != Some("the-token") {
            return StatusCode::UNAUTHORIZED;
        }
        let Ok(set) = serde_json::from_str::<Value>(&body) else {
            return StatusCode::BAD_REQUEST;
        };
        let kind = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        fake.taken.lock().unwrap().push((kind, set));
        StatusCode::NO_CONTENT
    }

    async fn capabilities(
        State(fake): State<Fake>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> (StatusCode, String) {
        let Some(token) = bearer(&headers) else {
            return (StatusCode::UNAUTHORIZED, String::new());
        };
        let key = if fake.is == Is::Forged {
            "not-the-token"
        } else {
            token
        };
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).unwrap();
        let nonce = uri
            .query()
            .and_then(|query| query.strip_prefix("nonce="))
            .unwrap_or_default();
        mac.update(format!("hotline-broker-v1|boot-1|{nonce}").as_bytes());
        let attest = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let mode = if fake.is == Is::Legacy {
            "legacy"
        } else {
            "split"
        };
        let body = json!({"version": "0.13.0", "boot": "boot-1", "protected": {"v": 1, "mode": mode}, "attest": attest});
        (StatusCode::OK, body.to_string())
    }

    /// A stand-in computer; `with_route` false is a release from before
    /// secrets, which knows no such path.
    async fn computer(with_route: bool, is: Is) -> (Ready, Fake, tokio::task::JoinHandle<()>) {
        let fake = Fake {
            is,
            taken: Arc::default(),
        };
        let mut app = axum::Router::new();
        if with_route {
            app = app.route("/secrets", put(secrets));
        }
        if is != Is::Old {
            app = app.route("/capabilities", get(capabilities));
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

    fn granted(secret: StoredSecret, readable: bool) -> Granted {
        Granted { secret, readable }
    }

    fn login() -> StoredSecret {
        StoredSecret::Login {
            r#use: crate::contract::SecretUse::Auto,
            sites: vec!["https://github.com".to_owned()],
            username: "george".to_owned(),
            password: "correct horse battery".to_owned(),
            totp: None,
        }
    }

    fn variable(value: &str, bound: bool) -> StoredSecret {
        StoredSecret::Variable {
            value: value.to_owned(),
            r#use: bound.then(|| {
                crate::vault::resolve_used_with(&crate::contract::UsedWith {
                    service: "github".into(),
                    host: None,
                })
                .unwrap()
            }),
        }
    }

    /// A readable variable, a key Used with GitHub, and a login.
    fn the_set() -> BTreeMap<String, Granted> {
        BTreeMap::from([
            (
                "NPM_TOKEN".to_owned(),
                granted(variable("npm_notarealtoken01", false), true),
            ),
            (
                "GITHUB_TOKEN".to_owned(),
                granted(
                    variable("ghp_notarealtoken00000000000000000000001", true),
                    false,
                ),
            ),
            ("GITHUB".to_owned(), granted(login(), false)),
        ])
    }

    #[tokio::test]
    async fn an_old_or_legacy_computer_is_handed_what_it_was_and_no_protected_key() {
        for is in [Is::Old, Is::Legacy] {
            let (ready, fake, serving) = computer(true, is).await;
            let handed = deliver(&ready, &the_set()).await.expect("taken");
            assert_eq!(
                handed,
                Handed {
                    protected: false,
                    waiting: vec!["GITHUB_TOKEN".into()],
                }
            );
            let taken = fake.taken.lock().unwrap().clone();
            // The v1 form, which every release takes: the variable as its
            // bare value, the login as its record, as 0.49 sent them.
            assert_eq!(taken.len(), 1);
            assert_ne!(taken[0].0.as_deref(), Some(V2));
            assert_eq!(taken[0].1["NPM_TOKEN"], "npm_notarealtoken01");
            assert_eq!(taken[0].1["GITHUB"]["kind"], "login");
            assert!(taken[0].1.get("GITHUB_TOKEN").is_none());
            serving.abort();
        }
    }

    #[tokio::test]
    async fn a_split_computer_that_attests_is_handed_every_grant_in_the_v2_form() {
        let (ready, fake, serving) = computer(true, Is::Split).await;
        let handed = deliver(&ready, &the_set()).await.expect("taken");
        assert_eq!(
            handed,
            Handed {
                protected: true,
                waiting: Vec::new()
            }
        );
        deliver(&ready, &BTreeMap::new()).await.expect("cleared");
        let taken = fake.taken.lock().unwrap().clone();
        assert_eq!(taken[0].0.as_deref(), Some(V2));
        let body = &taken[0].1;
        assert_eq!(body["v"], 2);
        assert!(taken[1].1["generation"].as_i64() > body["generation"].as_i64());
        assert_eq!(
            body["secrets"]["NPM_TOKEN"],
            json!({"kind": "variable", "value": "npm_notarealtoken01", "readable": true})
        );
        let key = &body["secrets"]["GITHUB_TOKEN"];
        assert_eq!(key["use"]["header"], "Authorization");
        assert!(
            key["use"]["origins"]
                .as_array()
                .unwrap()
                .contains(&json!("https://api.github.com"))
        );
        assert_eq!(
            key["use"]["standIn"],
            json!({"prefix": "ghp_", "length": 40})
        );
        assert!(key.get("readable").is_none());
        assert_eq!(body["secrets"]["GITHUB"]["kind"], "login");
        assert_eq!(taken[1].1["secrets"], json!({}), "an empty set clears it");
        serving.abort();
    }

    #[tokio::test]
    async fn a_forged_attest_gets_nothing() {
        let (ready, fake, serving) = computer(true, Is::Forged).await;
        let refused = deliver(&ready, &the_set()).await.unwrap_err();
        assert!(
            matches!(&refused, Refusal::Failed(reason) if reason.contains("did not prove")),
            "{refused:?}"
        );
        assert!(fake.taken.lock().unwrap().is_empty(), "not even a variable");
        serving.abort();
    }

    #[tokio::test]
    async fn a_release_from_before_secrets_is_told_apart_from_a_failure() {
        let (ready, _, serving) = computer(false, Is::Old).await;
        assert_eq!(
            deliver(&ready, &BTreeMap::new()).await,
            Err(Refusal::TooOld)
        );
        serving.abort();

        let (ready, fake, serving) = computer(true, Is::Old).await;
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

    #[tokio::test]
    async fn capture_delivery_failure_cannot_echo_values_into_the_desk_log() {
        let app = axum::Router::new().route(
            "/secrets",
            put(|body: String| async move { (StatusCode::BAD_REQUEST, body) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ready = Ready {
            url: format!("http://{}/mcp", listener.local_addr().unwrap()),
            token: "test-token".into(),
        };
        let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let secret = StoredSecret::Variable {
            value: "capture-private-value".into(),
            r#use: None,
        };
        let debug = format!("{secret:?}");
        let error = deliver(
            &ready,
            &BTreeMap::from([("GITHUB_TOKEN".into(), granted(secret, true))]),
        )
        .await
        .unwrap_err();
        assert!(matches!(&error, Refusal::Failed(reason) if reason.contains("400")));
        for sink in [debug, format!("{error:?}")] {
            assert!(!sink.contains("capture-private-value"));
        }
        serving.abort();
    }
}
