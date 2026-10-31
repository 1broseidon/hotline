//! The asides a teammate has been left, read off its own stream.
//!
//! Each is one `aside` line on [`StreamId::Asides`], written when it is left
//! and rewritten under the same id when a turn hears it, so the fold is the
//! whole state: what was said, and whether it has been heard. The room writes
//! them; this only reads.

use crate::log::{Log, StreamId};
use hotline_contract::{Aside, AsideSource, TranscriptEvent};
use serde_json::Value;

/// One aside as its line on a stream holds it, or nothing for any other line.
pub fn read(event: &Value) -> Option<Aside> {
    if event.get("kind").and_then(Value::as_str) != Some("aside") {
        return None;
    }
    match serde_json::from_value::<TranscriptEvent>(event.clone()).ok()? {
        TranscriptEvent::Aside(aside) => Some(aside),
        _ => None,
    }
}

/// Every aside the teammate has been left, oldest first.
pub fn of(log: &Log, persona_id: &str) -> Vec<Aside> {
    log.load(&StreamId::Asides(persona_id.to_string()))
        .iter()
        .filter_map(read)
        .collect()
}

/// How many no turn of the teammate's has heard yet.
pub fn unheard(log: &Log, persona_id: &str) -> usize {
    of(log, persona_id)
        .iter()
        .filter(|aside| aside.heard.is_none())
        .count()
}

/// The thread an aside came from, as a search hit names it: `pair:<key>`, or
/// `inbound:<source>` for a source outside the desk.
pub fn source_name(source: &AsideSource) -> String {
    match source {
        AsideSource::Thread(thread) => thread.to_string(),
        AsideSource::Inbound(id) => format!("inbound:{id}"),
    }
}
