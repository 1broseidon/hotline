//! The desks the window shows (BRO-145): this computer's, and every remote
//! desk it is paired with, each as an endpoint the window dials the same
//! way — the local Door, or the loopback bridge to a remote desk. The
//! window reads the list as `window.__hotlineDesks` (see `ui/src/desks.ts`).
//!
//! Remote desks come from the pairing registry once the client exists. Until
//! then, and for development, `HOTLINE_EXTRA_DESKS` adds desks by endpoint:
//! a JSON list of `{ "id", "name", "origin", "token" }`, for example a second
//! `hotline serve` on this machine, whose Door speaks the same wire contract
//! the bridge will.

use serde_json::{Value, json};

/// The local desk, then any extra ones, as the window reads them.
pub(crate) fn listed(origin: &str, token: &str) -> Value {
    let mut desks = vec![json!({
        "id": "local",
        "name": "This computer",
        "kind": "local",
        "origin": origin,
        "token": token,
    })];
    desks.extend(extra(std::env::var("HOTLINE_EXTRA_DESKS").ok().as_deref()));
    Value::Array(desks)
}

/// Desks from `HOTLINE_EXTRA_DESKS`. A malformed entry is skipped and said,
/// rather than stopping the window from opening.
fn extra(raw: Option<&str>) -> Vec<Value> {
    let Some(raw) = raw.filter(|raw| !raw.trim().is_empty()) else {
        return Vec::new();
    };
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(raw) else {
        eprintln!("[desks] HOTLINE_EXTRA_DESKS is not a JSON list; ignoring it");
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let field = |name: &str| entry.get(name).and_then(Value::as_str).map(str::to_string);
            match (field("id"), field("name"), field("origin"), field("token")) {
                (Some(id), Some(name), Some(origin), Some(token)) if id != "local" => Some(json!({
                    "id": id,
                    "name": name,
                    "kind": "remote",
                    "origin": origin,
                    "token": token,
                    "state": "open",
                })),
                _ => {
                    eprintln!("[desks] skipping an extra desk without id, name, origin and token");
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_desk_comes_first_and_extra_desks_are_remote() {
        let desks = extra(Some(
            r#"[{"id":"lab","name":"Lab","origin":"http://127.0.0.1:9","token":"t"},{"id":"broken"}]"#,
        ));
        assert_eq!(desks.len(), 1);
        assert_eq!(desks[0]["kind"], "remote");
        assert_eq!(desks[0]["id"], "lab");
        assert!(extra(Some("not json")).is_empty());
        assert!(extra(None).is_empty());
        // An extra desk cannot pose as the local one.
        assert!(
            extra(Some(
                r#"[{"id":"local","name":"x","origin":"o","token":"t"}]"#
            ))
            .is_empty()
        );
    }
}
