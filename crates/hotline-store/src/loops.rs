//! Living open loops a teammate carries across chapters, read off its own
//! stream (`loops/<personaId>.jsonl`).
//!
//! Each is one `open_loop` line on [`StreamId::Loops`], written when opened
//! and rewritten under the same id each time its summary or state changes, so
//! the fold is the current state.

use crate::log::{Log, StreamId};
use hotline_contract::{LoopState, OpenLoop};
use serde_json::Value;

/// The `kind` field on every line in [`StreamId::Loops`].
pub const KIND: &str = "open_loop";

/// One loop as its line on a stream holds it, or nothing for any other line
/// (including one written by a newer build with an unknown state).
pub fn read(event: &Value) -> Option<OpenLoop> {
    if event.get("kind").and_then(Value::as_str) != Some(KIND) {
        return None;
    }
    let mut item = serde_json::from_value::<OpenLoop>(event.clone()).ok()?;
    item.newer.remove("kind");
    item.newer.remove("ts");
    Some(item)
}

/// The line a loop is written as on [`StreamId::Loops`]. Carries `kind` and
/// `ts` beside the [`OpenLoop`] fields so the log fold and search index read
/// it uniformly.
pub fn line(item: &OpenLoop) -> Value {
    let mut value = serde_json::to_value(item).expect("OpenLoop serialises to JSON");
    if let Some(map) = value.as_object_mut() {
        map.insert("kind".to_string(), Value::String(KIND.to_string()));
        map.insert(
            "ts".to_string(),
            Value::Number(serde_json::Number::from(
                item.closed_at.unwrap_or(item.updated_at),
            )),
        );
    }
    value
}

fn state_rank(state: LoopState) -> u8 {
    match state {
        LoopState::Stale => 0,
        LoopState::Active => 1,
        LoopState::Waiting => 2,
        LoopState::Done => 3,
        LoopState::Archived => 4,
    }
}

/// Every loop kept for `persona_id`, ordered for display: stale first, then
/// active, waiting, recently done, archived; newest `updated_at` first within
/// each state.
pub fn of(log: &Log, persona_id: &str) -> Vec<OpenLoop> {
    let mut loops: Vec<OpenLoop> = log
        .load(&StreamId::Loops(persona_id.to_string()))
        .iter()
        .filter_map(read)
        .collect();
    loops.sort_by(|a, b| {
        state_rank(a.state)
            .cmp(&state_rank(b.state))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    loops
}

/// Open loops (`active`, `waiting`, `stale`) for `persona_id`.
pub fn open_of(log: &Log, persona_id: &str) -> Vec<OpenLoop> {
    of(log, persona_id)
        .into_iter()
        .filter(|item| item.state.is_open())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotline_contract::LoopSource;
    use tempfile::TempDir;

    fn sample(id: &str, key: &str, state: LoopState, updated_at: i64) -> OpenLoop {
        OpenLoop {
            id: id.into(),
            key: key.into(),
            summary: format!("Summary for {key}"),
            state,
            source: LoopSource::Chapter,
            created_at: updated_at,
            updated_at,
            review_by: None,
            job_id: None,
            last_chapter_id: None,
            closed_at: matches!(state, LoopState::Done | LoopState::Archived).then_some(updated_at),
            newer: serde_json::Map::new(),
        }
    }

    #[test]
    fn loops_fold_by_id_and_sort_stale_active_waiting_done_archived() {
        let dir = TempDir::new().unwrap();
        let log = Log::open(dir.path());
        let stream = StreamId::Loops("ada".into());
        log.append(
            &stream,
            &line(&sample("1", "wait", LoopState::Waiting, 100)),
        )
        .unwrap();
        log.append(&stream, &line(&sample("2", "act", LoopState::Active, 200)))
            .unwrap();
        log.append(&stream, &line(&sample("3", "old", LoopState::Stale, 50)))
            .unwrap();
        log.append(&stream, &line(&sample("4", "fin", LoopState::Done, 300)))
            .unwrap();
        // Rewrite id 1 to archived; latest line wins.
        log.append(
            &stream,
            &line(&sample("1", "wait", LoopState::Archived, 400)),
        )
        .unwrap();

        let keys: Vec<String> = of(&log, "ada").into_iter().map(|l| l.key).collect();
        assert_eq!(keys, vec!["old", "act", "fin", "wait"]);
        assert_eq!(open_of(&log, "ada").len(), 2);
    }
}
