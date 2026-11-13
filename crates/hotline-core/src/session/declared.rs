//! What a teammate says of its own work: what it is on (`activity`) and what
//! a turn left behind (`outcome`).
//!
//! The teammate writes the words and the desk draws them; the desk never
//! makes them up. Each call is one line on the thread it was made in, so
//! together they head the turn's path in the pane and survive a reload, and
//! each returns at once: saying what you are on must never slow the work.
//!
//! An open outcome (a server left up, replies still coming) is the one that
//! changes later. The teammate names it with `id`, and the same `id` again
//! rewrites that line where it stands, under the turn that made it: still
//! `open` with new words, closed as what it became, or `ended` with nothing
//! left behind. The line is looked for on the thread the call is made in and
//! then in the teammate's main conversation, so a job that watches for the
//! replies can close what the conversation opened. The desk closes nothing
//! by itself.

use super::{Room, new_id, now_ms};
use crate::contract::{ThreadId, TranscriptEvent};
use crate::log::StreamId;
use serde_json::Value;
use std::fmt::Write;

/// The kinds of activity the mark has a movement for.
pub(crate) const ACTIVITY_TYPES: [&str; 5] =
    ["thinking", "reading", "searching", "making", "talking"];

/// The kinds of outcome the line has a mark for.
pub(crate) const OUTCOME_TYPES: [&str; 5] = ["sent", "made", "changed", "open", "yours"];

/// How an outcome did, when it did not wholly work.
pub(crate) const OUTCOME_STATES: [&str; 3] = ["done", "failed", "partial"];

/// A few plain words beside the mark, never a report.
pub(crate) const ACTIVITY_CHARS: usize = 80;

/// One line under a reply.
pub(crate) const OUTCOME_CHARS: usize = 120;

/// A teammate's own name for an outcome.
const KEY_CHARS: usize = 64;

/// A word from the arguments, trimmed, or none when absent or blank.
fn word<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Words of at most `most` characters, or why not.
fn short(tool: &str, field: &str, text: &str, most: usize) -> Result<(), String> {
    if text.chars().count() > most {
        return Err(format!(
            "{tool}'s `{field}` is at most {most} characters: a few plain words."
        ));
    }
    Ok(())
}

/// What a kind the desk does not know comes back with: it is kept, and
/// drawn plainly.
fn unknown(kind: &str, known: &[&str]) -> String {
    format!(
        " The desk has no mark for '{kind}', so it is shown plainly; the kinds are {}.",
        known.join(", ")
    )
}

impl Room {
    /// Writes what the teammate says it is on, on the thread it is working
    /// in.
    pub(crate) fn declare_activity(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        arguments: &Value,
    ) -> Result<String, String> {
        let kind = word(arguments, "type")
            .ok_or("activity needs a `type`: thinking, reading, searching, making or talking.")?
            .to_lowercase();
        let message = word(arguments, "message");
        if let Some(message) = message {
            short("activity", "message", message, ACTIVITY_CHARS)?;
        }
        let event = TranscriptEvent::Activity {
            id: new_id(),
            ts: now_ms(),
            r#type: kind.clone(),
            message: message.map(str::to_string),
        };
        self.threads().write(thread, persona_id, &event);
        let mut said = "Noted.".to_string();
        if !ACTIVITY_TYPES.contains(&kind.as_str()) {
            said.push_str(&unknown(&kind, &ACTIVITY_TYPES));
        }
        Ok(said)
    }

    /// Writes something this turn left behind, or rewrites an open one the
    /// teammate named: see the module.
    pub(crate) fn declare_outcome(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        arguments: &Value,
    ) -> Result<String, String> {
        let kind = word(arguments, "type")
            .ok_or("outcome needs a `type`: sent, made, changed, open or yours.")?
            .to_lowercase();
        let message = word(arguments, "message")
            .ok_or("outcome needs a `message`: what was left behind, in a few words.")?;
        short("outcome", "message", message, OUTCOME_CHARS)?;
        let state = word(arguments, "state").map(str::to_lowercase);
        let key = word(arguments, "id");
        if let Some(key) = key {
            short("outcome", "id", key, KEY_CHARS)?;
        }
        let ended = arguments
            .get("ended")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let open = key.and_then(|key| self.open_outcome(persona_id, thread, key));
        if ended && open.is_none() {
            return Err(format!(
                "There is no open outcome called '{}' to end.",
                key.unwrap_or_default()
            ));
        }
        let now = now_ms();
        let (id, ts, closed_at, written_to) = match &open {
            Some((line, on)) => (
                line["id"].as_str().unwrap_or_default().to_string(),
                line["ts"].as_i64().unwrap_or(now),
                // Closed: ended, or now something other than open.
                (ended || kind != "open").then_some(now),
                on.clone(),
            ),
            None => (new_id(), now, None, thread.clone()),
        };
        let key = key.map_or_else(|| id[..8.min(id.len())].to_string(), str::to_string);
        let event = TranscriptEvent::Outcome {
            id,
            ts,
            // An ended one keeps its kind: it was open, and nothing is left.
            r#type: if ended {
                "open".to_string()
            } else {
                kind.clone()
            },
            message: message.to_string(),
            state: state.clone().filter(|state| state != "done"),
            key: key.clone(),
            closed_at,
        };
        self.threads().write(&written_to, persona_id, &event);
        let mut said = match (&open, ended, kind.as_str()) {
            (Some(_), true, _) => "Ended. It has left the line.".to_string(),
            (Some(_), false, "open") => "Updated.".to_string(),
            (Some(_), false, _) => "Closed.".to_string(),
            (None, _, "open") => {
                format!("Noted as open, id '{key}'. When it ends, call outcome again with this id.")
            }
            (None, _, _) => "Noted.".to_string(),
        };
        if !OUTCOME_TYPES.contains(&kind.as_str()) {
            said.push_str(&unknown(&kind, &OUTCOME_TYPES));
        }
        if let Some(state) = state.filter(|state| !OUTCOME_STATES.contains(&state.as_str())) {
            let _ = write!(
                said,
                " '{state}' is not a state; it is shown as done. The states are failed and partial."
            );
        }
        Ok(said)
    }

    /// The open outcome called `key`, as its line stands, and the thread it
    /// is on: this thread's first, then the main conversation's.
    fn open_outcome(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        key: &str,
    ) -> Option<(Value, ThreadId)> {
        let home = ThreadId::dm(persona_id);
        let mut places = vec![thread.clone()];
        if *thread != home {
            places.push(home);
        }
        places.into_iter().find_map(|place| {
            let stream = StreamId::of_thread(&place)?;
            let line = self.log.load(&stream).into_iter().rev().find(|event| {
                event["kind"] == "outcome"
                    && event["key"] == key
                    && event["type"] == "open"
                    && event.get("closedAt").is_none()
            })?;
            Some((line, place))
        })
    }
}
