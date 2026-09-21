//! The one moment a teammate's passkey is made, driven from the desk.
//!
//! A passkey is a WebAuthn credential the computer's browser keeps in a
//! virtual authenticator and signs in with by itself; its private key is
//! made in the browser and never typed. Making one is the operator's act:
//! the desk arms the computer for one site, for ten minutes, through
//! `PUT /passkeys/registration`; the person then adds a passkey in the
//! site's own security settings through the computer's screen, or asks the
//! teammate to press the button; the desk polls `GET` until the computer
//! answers the minted credential, stores it in the vault, hands the computer
//! its set with it, and ends the arming with `DELETE`. That answer is the
//! one time a private key leaves the computer, over the same bearer-guarded
//! loopback door the secrets go in by, in the direction the cookie import
//! already trusts.

use super::Ready;
use super::secrets::Refusal;
use crate::vault::StoredSecret;
use serde_json::{Value, json};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Where the computer says the making of a passkey stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Registration {
    Idle,
    Armed {
        rp_id: String,
        expires_at: i64,
    },
    /// The credential the browser minted under the arming, as a record the
    /// vault stores as it is.
    Registered {
        rp_id: String,
        credential: StoredSecret,
    },
}

fn client() -> Result<reqwest::Client, Refusal> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Refusal::Failed(error.to_string()))
}

fn door(ready: &Ready) -> String {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    format!("{base}/passkeys/registration")
}

async fn answered(response: reqwest::Response) -> Result<Value, Refusal> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Refusal::TooOld);
    }
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let reason = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| body.trim().to_owned());
        return Err(Refusal::Failed(if reason.is_empty() {
            format!("The computer refused ({status}).")
        } else {
            format!("The computer refused ({status}): {reason}")
        }));
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body)
        .map_err(|error| Refusal::Failed(format!("The computer answered something else: {error}")))
}

fn registration(answer: &Value) -> Result<Registration, Refusal> {
    let rp_id = || answer["rpId"].as_str().unwrap_or_default().to_owned();
    match answer["state"].as_str() {
        Some("idle") => Ok(Registration::Idle),
        Some("armed") => Ok(Registration::Armed {
            rp_id: rp_id(),
            expires_at: answer["expiresAt"].as_i64().unwrap_or_default(),
        }),
        Some("registered") => {
            let credential = serde_json::from_value::<StoredSecret>(answer["credential"].clone())
                .map_err(|error| {
                Refusal::Failed(format!(
                    "The computer answered a passkey the desk cannot store: {error}"
                ))
            })?;
            if !matches!(credential, StoredSecret::Passkey { .. }) {
                return Err(Refusal::Failed(
                    "The computer answered something that is not a passkey.".to_owned(),
                ));
            }
            Ok(Registration::Registered {
                rp_id: rp_id(),
                credential,
            })
        }
        other => Err(Refusal::Failed(format!(
            "The computer answered an unknown registration state: {other:?}."
        ))),
    }
}

/// Arms the computer for `rp_id`, replacing any earlier arming, and answers
/// when the arming ends, ms since the epoch.
pub async fn arm(ready: &Ready, rp_id: &str) -> Result<i64, Refusal> {
    let response = client()?
        .put(door(ready))
        .bearer_auth(&ready.token)
        .json(&json!({"rpId": rp_id}))
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    match registration(&answered(response).await?)? {
        Registration::Armed { expires_at, .. } => Ok(expires_at),
        other => Err(Refusal::Failed(format!(
            "The computer did not arm: {other:?}."
        ))),
    }
}

/// Where the making stands now; the computer looks at its browser to answer.
pub async fn poll(ready: &Ready) -> Result<Registration, Refusal> {
    let response = client()?
        .get(door(ready))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    registration(&answered(response).await?)
}

/// Ends the arming. What it minted stays in the browser only if the set
/// delivered since carries it.
pub async fn disarm(ready: &Ready) -> Result<(), Refusal> {
    let response = client()?
        .delete(door(ready))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    answered(response).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::guide::fake;

    #[tokio::test]
    async fn an_arming_is_put_polled_and_ended_over_the_bearer_door() {
        let (port, taken) = fake::serve_taking(Some("0.8.0")).await;
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "the-token".to_owned(),
        };
        assert_eq!(poll(&ready).await, Ok(Registration::Idle));
        let expires_at = arm(&ready, "github.com").await.expect("armed");
        assert!(expires_at > 0);
        assert_eq!(taken.armed().as_deref(), Some("github.com"));
        assert_eq!(
            poll(&ready).await,
            Ok(Registration::Armed {
                rp_id: "github.com".to_owned(),
                expires_at
            })
        );
        taken.mint();
        match poll(&ready).await.expect("registered") {
            Registration::Registered { rp_id, credential } => {
                assert_eq!(rp_id, "github.com");
                assert!(
                    matches!(credential, StoredSecret::Passkey { ref rp_id, .. } if rp_id == "github.com")
                );
            }
            other => panic!("{other:?}"),
        }
        disarm(&ready).await.expect("ended");
        assert_eq!(taken.armed(), None);
        assert_eq!(poll(&ready).await, Ok(Registration::Idle));

        // A site that is not one is refused by the computer, and said so.
        let refused = arm(&ready, "GitHub.com").await.unwrap_err();
        assert!(
            matches!(&refused, Refusal::Failed(reason) if reason.contains("400")),
            "{refused:?}"
        );
        // A release from before passkeys has no such door.
        let (port, _) = fake::serve_taking(Some("0.7.0")).await;
        let old = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "the-token".to_owned(),
        };
        assert_eq!(arm(&old, "github.com").await, Err(Refusal::TooOld));
        // A refused bearer is a refusal, not an old release. (The fake
        // checks that a bearer is presented, not which.)
        let wrong = Ready {
            token: String::new(),
            ..ready
        };
        assert!(matches!(
            arm(&wrong, "github.com").await,
            Err(Refusal::Failed(reason)) if reason.contains("401")
        ));
    }
}
