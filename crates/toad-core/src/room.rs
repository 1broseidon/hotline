//! The room, folded out of its stream: who is on the team, and how the room
//! is set.
//!
//! There is no roster table and no settings file. Everything the room
//! remembers is an event on [`StreamId::Room`], shaped `{kind, id, …}`, and
//! these two functions are the whole of reading it back:
//!
//! - `persona` carries a teammate's record — the [`Persona`] fields beside the
//!   kind, `id` being the teammate's — and [`roster`] is every one of them
//!   that is not deleted.
//! - `setting` carries one preference, `id` naming it and `value` holding it,
//!   and [`settings`] is those laid over the room's defaults.
//!
//! A delete is not a new kind: it is the same kind and id again with
//! `deleted: true`. The stream folds by id, so the tombstone is the last word
//! and a reader finds it instead of what it replaced — which is also what
//! makes a delete something a mirror can ship, rather than an absence it has
//! to notice.

use crate::contract::{Persona, SessionCheckpoint};
use crate::log::{Log, StreamId};
use serde_json::{Map, Value, json};

/// What a setting means before anybody has set it. `pi` is the built-in Toad
/// Agent, which is what a new teammate runs on, and a chapter closes after
/// eight hours of quiet — a working day's gap, so yesterday's context does not
/// follow you into this morning.
fn defaults() -> Map<String, Value> {
    let mut settings = Map::new();
    settings.insert("defaultBackendId".into(), Value::from("pi"));
    settings.insert("chapterIdleHours".into(), Value::from(8));
    settings
}

fn is_kind(event: &Value, kind: &str) -> bool {
    event.get("kind").and_then(Value::as_str) == Some(kind)
}

fn is_deleted(event: &Value) -> bool {
    event.get("deleted").and_then(Value::as_bool) == Some(true)
}

/// The team, in the order its teammates first appeared on the stream.
///
/// An event that does not read as a `Persona` is skipped rather than fatal: a
/// line a newer build wrote, or one a half-written record left behind, costs
/// its own teammate and not the whole roster.
pub fn roster(log: &Log) -> Vec<Persona> {
    log.load(&StreamId::Room)
        .into_iter()
        .filter(|event| is_kind(event, "persona") && !is_deleted(event))
        .filter_map(|event| serde_json::from_value(event).ok())
        .collect()
}

/// Every setting the room has, the ones nobody set included.
///
/// A deleted setting is not an override, so its default stands again — which
/// is what "delete" means to somebody clearing a preference.
pub fn settings(log: &Log) -> Map<String, Value> {
    let mut settings = defaults();
    for event in log.load(&StreamId::Room) {
        if !is_kind(&event, "setting") || is_deleted(&event) {
            continue;
        }
        let Some(key) = event.get("id").and_then(Value::as_str) else {
            continue;
        };
        let value = event.get("value").cloned().unwrap_or(Value::Null);
        settings.insert(key.to_string(), value);
    }
    settings
}

/// A persona event is the teammate's record with the kind beside it, and the
/// whole record every time: a stream folds by id, so a line carrying only what
/// changed would leave the fold holding only what changed.
pub(crate) fn append_persona(log: &Log, persona: &Persona) -> Result<(), String> {
    let mut event = json!(persona);
    event
        .as_object_mut()
        .expect("a teammate serializes as an object")
        .insert("kind".into(), Value::from("persona"));
    log.append(&StreamId::Room, &event)
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// Remembers an agent's own id for this teammate's conversation, so a later
/// session can ask that same backend to reopen it.
///
/// One entry per backend, replaced in place: a teammate that moves between
/// harnesses and back finds both conversations where it left them, and Cursor
/// is never handed Claude's session id.
pub(crate) fn checkpoint_session(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
    session_id: &str,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| checkpoint.backend_id != backend_id);
        checkpoints.push(SessionCheckpoint {
            backend_id: backend_id.to_string(),
            session_id: session_id.to_string(),
        });
    })
}

/// Withdraws the promise to reopen that backend's session. The session itself
/// is not touched; what is gone is Toad's intention to return to it, which is
/// what closing a chapter means.
pub(crate) fn clear_checkpoint(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| checkpoint.backend_id != backend_id);
    })
}

/// The teammate's record with its checkpoints changed and nothing else, or
/// nothing at all when they did not change.
fn with_checkpoints(
    log: &Log,
    persona_id: &str,
    change: impl FnOnce(&mut Vec<SessionCheckpoint>),
) -> Result<(), String> {
    let mut persona = roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .ok_or_else(|| format!("There is no teammate {persona_id}."))?;
    let before = persona.session_checkpoints.clone();
    change(&mut persona.session_checkpoints);
    if persona.session_checkpoints == before {
        return Ok(());
    }
    persona.updated_at = chrono::Utc::now().timestamp_millis();
    append_persona(log, &persona)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{McpPolicy, PolicyMode};
    use serde_json::json;

    fn scratch(name: &str) -> Log {
        let root =
            std::env::temp_dir().join(format!("toad-core-room-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Log::open(root)
    }

    fn persona(id: &str, name: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: name.to_string(),
            goal: "Keep the harbour running.".to_string(),
            face: None,
            team: None,
            backend_id: "pi".to_string(),
            cwd: "/tmp/harbour".to_string(),
            reach: None,
            model_id: None,
            mode_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::All,
                server_ids: Vec::new(),
            },
            web_search_policy: None,
            computer: None,
            subagents: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    /// A persona event is the teammate's record with the kind beside it.
    fn persona_event(persona: &Persona) -> Value {
        let mut event = serde_json::to_value(persona).unwrap();
        let fields = event.as_object_mut().unwrap();
        fields.insert("kind".into(), Value::from("persona"));
        event
    }

    fn setting(key: &str, value: Value) -> Value {
        json!({"kind": "setting", "id": key, "value": value})
    }

    fn tombstone(kind: &str, id: &str) -> Value {
        json!({"kind": kind, "id": id, "deleted": true})
    }

    fn append(log: &Log, event: &Value) {
        log.append(&StreamId::Room, event).unwrap();
    }

    #[test]
    fn a_persona_event_is_the_contracts_persona_beside_its_kind() {
        let log = scratch("persona-round-trip");
        let ada = persona("ada", "Ada");
        let written = persona_event(&ada);
        append(&log, &written);

        // Out of the stream and back into the struct the wire is defined by,
        // and out again as the same bytes: a field spelled differently here
        // than the contract spells it fails on one side or the other.
        assert_eq!(roster(&log), [ada]);
        assert_eq!(log.load(&StreamId::Room), [written]);
    }

    #[test]
    fn the_roster_is_the_living_teammates_in_the_order_they_arrived() {
        let log = scratch("roster");
        let ada = persona("ada", "Ada");
        let bob = persona("bob", "Bob");
        append(&log, &persona_event(&ada));
        append(&log, &persona_event(&bob));

        // A rename is the same id again, and keeps the place it arrived in.
        let mut renamed = ada.clone();
        renamed.name = "Ada Lovelace".to_string();
        renamed.updated_at = 1_700_000_001_000;
        append(&log, &persona_event(&renamed));

        assert_eq!(roster(&log), [renamed, bob]);
    }

    #[test]
    fn a_tombstone_is_the_last_word_on_a_teammate() {
        let log = scratch("tombstone");
        let ada = persona("ada", "Ada");
        let bob = persona("bob", "Bob");
        append(&log, &persona_event(&ada));
        append(&log, &persona_event(&bob));
        append(&log, &tombstone("persona", "ada"));

        assert_eq!(roster(&log), [bob]);
        // The stream still carries the delete, which is what a mirror ships.
        let ids: Vec<String> = log
            .load(&StreamId::Room)
            .iter()
            .map(|event| event["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["ada", "bob"]);
    }

    #[test]
    fn a_setting_nobody_set_is_the_default_and_a_set_one_stands_over_it() {
        let log = scratch("settings");
        assert_eq!(settings(&log)["defaultBackendId"], "pi");
        assert_eq!(settings(&log)["chapterIdleHours"], 8);

        append(&log, &setting("chapterIdleHours", Value::from(2)));
        append(&log, &setting("theme", Value::from("dark")));
        assert_eq!(settings(&log)["chapterIdleHours"], 2);
        assert_eq!(settings(&log)["theme"], "dark");
        assert_eq!(settings(&log)["defaultBackendId"], "pi");

        // Clearing a setting is a tombstone, and puts the default back. A
        // setting that never had one is simply gone.
        append(&log, &tombstone("setting", "chapterIdleHours"));
        append(&log, &tombstone("setting", "theme"));
        assert_eq!(settings(&log)["chapterIdleHours"], 8);
        assert!(!settings(&log).contains_key("theme"));
    }
}
