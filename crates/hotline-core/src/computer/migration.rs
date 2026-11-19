//! What a computer's first split boot leaves for the person: the snapshots
//! taken before it, which may hold the old sign-ins and are not restored
//! until the person says what becomes of them. The computer has already
//! dropped the saved logins and cleared the signed-in sites from its general
//! browser; the snapshots wait for an answer, and nothing in them is erased
//! without one.

use super::Ready;
use crate::contract::{ComputerMigration, SnapshotAnswer};
use serde_json::{Value, json};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// The snapshots waiting for the person, or none: a computer with nothing
/// pending, one already answered, and one with no such door (before 0.13,
/// or legacy) all have nothing to ask.
pub async fn pending(ready: &Ready) -> Result<Option<ComputerMigration>, String> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let client = super::door_client()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(format!("{base}/migration"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| format!("Could not reach the computer: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "The computer would not say what its update left ({}).",
            response.status()
        ));
    }
    let answer: Value = response
        .json()
        .await
        .map_err(|_| "The computer's answer could not be read.".to_string())?;
    if answer["state"] != "pending" {
        return Ok(None);
    }
    serde_json::from_value(json!({"snapshots": answer["snapshots"]}))
        .map(Some)
        .map_err(|_| "The computer's snapshots could not be read.".to_string())
}

/// Sends the person's answer for the snapshots.
pub async fn answer(ready: &Ready, snapshots: SnapshotAnswer) -> Result<(), String> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let client = super::door_client()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{base}/migration/answer"))
        .bearer_auth(&ready.token)
        .json(&json!({"snapshots": snapshots}))
        .send()
        .await
        .map_err(|error| format!("Could not reach the computer: {error}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "The computer did not take the answer ({}).",
            response.status()
        ))
    }
}
