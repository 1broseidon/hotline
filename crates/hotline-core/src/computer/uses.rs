//! The computer owns held uses and their ten-minute allowances. The desk
//! carries only metadata and authenticated answers across this door.
use super::Ready;
use crate::contract::{CaptureKind, SecretUseAnswer};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeldUse {
    pub id: String,
    pub secret: String,
    pub kind: CaptureKind,
    pub site: String,
    pub path: String,
    pub asked_at: i64,
    pub expires_at: i64,
    /// From a protected computer: what the use is, and whether its broker's
    /// own sign-in step asked.
    #[serde(default)]
    pub operation: Option<crate::contract::SecretOperation>,
    #[serde(default)]
    pub attested: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    TooOld,
    Expired,
    Failed,
}

async fn request(
    ready: &Ready,
    answer: Option<(&str, SecretUseAnswer)>,
) -> Result<reqwest::Response, Refusal> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(&ready.url);
    let client = super::door_client()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| Refusal::Failed)?;
    let request = match answer {
        Some((id, answer)) => client
            .post(format!("{base}/secrets/uses/answer"))
            .json(&serde_json::json!({"id": id, "answer": answer})),
        None => client.get(format!("{base}/secrets/uses")),
    };
    let response = request
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|_| Refusal::Failed)?;
    match response.status() {
        reqwest::StatusCode::OK => Ok(response),
        reqwest::StatusCode::NOT_FOUND => Err(Refusal::TooOld),
        reqwest::StatusCode::CONFLICT => Err(Refusal::Expired),
        _ => Err(Refusal::Failed),
    }
}

/// A sign-in the computer's own step could not finish, waiting for the
/// person on the site's screen: `{kind: "finish", name, origin, account,
/// since}` from `GET /signin/pending`. The computer lists one 30 s after a
/// secret was typed while the tab still shows that document, and clears it
/// on a new document, the tab closing or a revoke.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Finish {
    pub name: String,
    pub origin: String,
    #[serde(default)]
    pub account: String,
    pub since: i64,
}

impl Finish {
    /// The one id the desk keeps a card under: the same sign-in polled
    /// again is the same card.
    pub(crate) fn id(&self) -> String {
        format!("finish:{}:{}:{}", self.name, self.origin, self.since)
    }
}

/// The uses the computer holds for an answer. An entry of a kind this desk
/// does not know is passed over, so a newer computer's addition never
/// stops the cards it does know.
pub(crate) async fn held(ready: &Ready) -> Result<Vec<HeldUse>, Refusal> {
    #[derive(Deserialize)]
    struct Uses {
        uses: Vec<serde_json::Value>,
    }
    let uses = request(ready, None)
        .await?
        .json::<Uses>()
        .await
        .map_err(|_| Refusal::Failed)?
        .uses;
    uses.into_iter()
        .filter(|entry| matches!(entry["kind"].as_str(), Some("login" | "passkey")))
        .map(|entry| serde_json::from_value(entry).map_err(|_| Refusal::Failed))
        .collect()
}

/// The sign-ins waiting for the person to finish them, from `GET
/// /signin/pending`. A computer without the door (before 0.13, or legacy)
/// has none; an entry of another kind is passed over.
pub(crate) async fn pending_signins(ready: &Ready) -> Result<Vec<Finish>, Refusal> {
    #[derive(Deserialize)]
    struct Pending {
        pending: Vec<serde_json::Value>,
    }
    let base = ready.url.strip_suffix("/mcp").unwrap_or(&ready.url);
    let client = super::door_client()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| Refusal::Failed)?;
    let response = client
        .get(format!("{base}/signin/pending"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|_| Refusal::Failed)?;
    match response.status() {
        reqwest::StatusCode::OK => {}
        reqwest::StatusCode::NOT_FOUND => return Ok(Vec::new()),
        _ => return Err(Refusal::Failed),
    }
    let pending = response
        .json::<Pending>()
        .await
        .map_err(|_| Refusal::Failed)?
        .pending;
    pending
        .into_iter()
        .filter(|entry| entry["kind"] == "finish")
        .map(|entry| serde_json::from_value(entry).map_err(|_| Refusal::Failed))
        .collect()
}

pub(crate) async fn answer(
    ready: &Ready,
    id: &str,
    answer: SecretUseAnswer,
) -> Result<(), Refusal> {
    #[derive(Deserialize)]
    struct Answer {
        state: String,
    }
    let body = request(ready, Some((id, answer)))
        .await?
        .json::<Answer>()
        .await
        .map_err(|_| Refusal::Failed)?;
    let expected = if answer == SecretUseAnswer::Deny {
        "denied"
    } else {
        "approved"
    };
    if body.state == expected {
        Ok(())
    } else {
        Err(Refusal::Failed)
    }
}

#[cfg(test)]
pub(crate) mod fake;
