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
/// since}` among the held uses. Gone from the list once it is finished or
/// given up.
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

/// What the computer holds: uses waiting for an answer, and sign-ins
/// waiting for the person to finish them. An entry of a kind this desk
/// does not know is passed over, so a newer computer's addition never
/// stops the cards it does know.
pub(crate) async fn held(ready: &Ready) -> Result<(Vec<HeldUse>, Vec<Finish>), Refusal> {
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
    let mut held = Vec::new();
    let mut finishing = Vec::new();
    for entry in uses {
        match entry["kind"].as_str() {
            Some("finish") => {
                finishing.push(serde_json::from_value(entry).map_err(|_| Refusal::Failed)?);
            }
            Some("login" | "passkey") => {
                held.push(serde_json::from_value(entry).map_err(|_| Refusal::Failed)?);
            }
            _ => {}
        }
    }
    Ok((held, finishing))
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
