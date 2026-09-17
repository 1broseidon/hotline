//! Rewrites `crates/hotline-core/models.json` from models.dev.
//!
//! ```text
//! cargo run -p hotline-core --bin hotline-models-sync            # fetch, filter, write
//! cargo run -p hotline-core --bin hotline-models-sync -- --from api.json
//! ```
//!
//! What it keeps is decided in `hotline_core::models`; this is only the fetch,
//! the write, and a list of what changed against the snapshot this binary
//! was built with, so the diff can be read before it is committed.

use hotline_core::models::{self, Catalog};
use std::collections::BTreeSet;
use std::process::ExitCode;

const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/models.json");

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("hotline-models-sync: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let body = match (args.next().as_deref(), args.next()) {
        (None, None) => reqwest::get(models::SOURCE)
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| format!("{}: {error}", models::SOURCE))?
            .text()
            .await
            .map_err(|error| error.to_string())?,
        (Some("--from"), Some(path)) => {
            std::fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?
        }
        _ => return Err("usage: hotline-models-sync [--from api.json]".to_string()),
    };
    let api: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| format!("not JSON: {error}"))?;
    let synced = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let fresh = models::snapshot(&api, &synced)?;

    report(models::catalog(), &fresh);
    let text = serde_json::to_string_pretty(&fresh).map_err(|error| error.to_string())?;
    std::fs::write(OUT, text + "\n").map_err(|error| format!("{OUT}: {error}"))?;
    println!("wrote {OUT}");
    Ok(())
}

/// Which models each provider gained and lost since the built-in snapshot.
fn report(before: &Catalog, after: &Catalog) {
    for (id, entry) in &after.providers {
        let now: BTreeSet<&String> = entry.models.keys().collect();
        let then: BTreeSet<&String> = before
            .providers
            .get(id)
            .map(|entry| entry.models.keys().collect())
            .unwrap_or_default();
        let added: Vec<&str> = now.difference(&then).map(|id| id.as_str()).collect();
        let removed: Vec<&str> = then.difference(&now).map(|id| id.as_str()).collect();
        println!("{id}: {} models", now.len());
        for model in added {
            println!("  + {model}");
        }
        for model in removed {
            println!("  - {model}");
        }
    }
    for id in before.providers.keys() {
        if !after.providers.contains_key(id) {
            println!("{id}: gone");
        }
    }
}
