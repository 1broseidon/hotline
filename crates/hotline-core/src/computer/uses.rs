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
    let client = reqwest::Client::builder()
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

pub(crate) async fn held(ready: &Ready) -> Result<Vec<HeldUse>, Refusal> {
    #[derive(Deserialize)]
    struct Uses {
        uses: Vec<HeldUse>,
    }
    request(ready, None)
        .await?
        .json::<Uses>()
        .await
        .map(|body| body.uses)
        .map_err(|_| Refusal::Failed)
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
