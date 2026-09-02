//! Chapters are a view of the tape, not a second file.
//!
//! A chapter is a marker event in the tape it divides. Closing supersedes
//! the marker by id, so the transcript stays the one record. The drawer
//! lists those markers newest first, each with how many things were said
//! in its slice — and a tape written before chapters existed has no
//! markers, so it lists as empty.

use crate::log::{Log, StreamId};
use serde_json::{Map, Value, json};

fn is_chapter(event: &Value) -> bool {
    event.get("kind").and_then(Value::as_str) == Some("chapter")
}

fn is_message(event: &Value) -> bool {
    matches!(
        event.get("kind").and_then(Value::as_str),
        Some("user" | "agent")
    )
}

/// Every marker in the tape, oldest first.
pub(crate) fn chapters_of(events: &[Value]) -> Vec<&Value> {
    events.iter().filter(|event| is_chapter(event)).collect()
}

/// The chapter that has not closed, if any. There is at most one, and it is
/// always the last: closing supersedes the marker by id rather than adding one.
pub(crate) fn open_chapter(events: &[Value]) -> Option<&Value> {
    chapters_of(events)
        .last()
        .copied()
        .filter(|chapter| chapter.get("endedAt").is_none())
}

/// Everything said or done within a chapter, the marker itself excluded.
pub(crate) fn slice_of<'a>(events: &'a [Value], chapter: &Value) -> &'a [Value] {
    let Some(chapter_id) = chapter.get("id").and_then(Value::as_str) else {
        return &[];
    };
    let Some(start) = events
        .iter()
        .position(|event| event.get("id").and_then(Value::as_str) == Some(chapter_id))
    else {
        return &[];
    };
    let after = &events[start + 1..];
    match after.iter().position(is_chapter) {
        Some(end) => &after[..end],
        None => after,
    }
}

/// Optional fields stay absent when the marker never had them, so the
/// JSON matches what Bun emits for the same tape.
pub fn summarize(events: &[Value]) -> Vec<Value> {
    let mut summaries: Vec<Value> = chapters_of(events)
        .into_iter()
        .map(|chapter| {
            let messages = slice_of(events, chapter)
                .iter()
                .filter(|event| is_message(event))
                .count();
            let mut summary = Map::new();
            if let Some(id) = chapter.get("id") {
                summary.insert("id".into(), id.clone());
            }
            if let Some(ts) = chapter.get("ts") {
                summary.insert("startedAt".into(), ts.clone());
            }
            for key in ["endedAt", "title", "note", "status"] {
                if let Some(value) = chapter.get(key) {
                    summary.insert(key.to_string(), value.clone());
                }
            }
            summary.insert("messages".into(), json!(messages));
            Value::Object(summary)
        })
        .collect();
    summaries.reverse();
    summaries
}

pub fn list(log: &Log, persona_id: &str) -> Vec<Value> {
    summarize(&log.load(&StreamId::Tape(persona_id.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::transcript_segments_dir;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> (PathBuf, Log) {
        let dir =
            std::env::temp_dir().join(format!("toad-core-chapters-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("transcripts")).unwrap();
        let log = Log::open(&dir);
        (dir, log)
    }

    fn write_tape(root: &Path, persona_id: &str, events: &[Value]) {
        let dir = transcript_segments_dir(root, persona_id);
        fs::create_dir_all(&dir).unwrap();
        let lines: Vec<String> = events.iter().map(ToString::to_string).collect();
        fs::write(dir.join("1.jsonl"), lines.join("\n")).unwrap();
    }

    #[test]
    fn an_empty_tape_lists_no_chapters() {
        let (_root, log) = scratch("empty");
        assert!(list(&log, "nobody").is_empty());
    }

    #[test]
    fn a_tape_with_no_markers_lists_no_chapters() {
        let (root, log) = scratch("no-markers");
        write_tape(
            &root,
            "p",
            &[
                json!({"kind": "user", "id": "u1", "ts": 1, "text": "hi"}),
                json!({"kind": "agent", "id": "a1", "ts": 2, "text": "hello"}),
            ],
        );
        assert!(list(&log, "p").is_empty());
    }

    #[test]
    fn two_chapters_newest_first_with_message_counts() {
        let (root, log) = scratch("two");
        write_tape(
            &root,
            "p",
            &[
                json!({
                    "kind": "chapter",
                    "id": "c1",
                    "ts": 100,
                    "endedAt": 200,
                    "title": "First",
                    "note": "did the thing",
                    "status": "done"
                }),
                json!({"kind": "user", "id": "u1", "ts": 110, "text": "hi"}),
                json!({"kind": "tool", "id": "t1", "ts": 120, "status": "done"}),
                json!({"kind": "agent", "id": "a1", "ts": 130, "text": "hello"}),
                json!({"kind": "chapter", "id": "c2", "ts": 300}),
                json!({"kind": "user", "id": "u2", "ts": 310, "text": "next"}),
                json!({"kind": "tool", "id": "t2", "ts": 320, "status": "pending"}),
                json!({"kind": "agent", "id": "a2", "ts": 330, "text": "ok"}),
                json!({"kind": "user", "id": "u3", "ts": 340, "text": "more"}),
            ],
        );

        assert_eq!(
            list(&log, "p"),
            vec![
                json!({
                    "id": "c2",
                    "startedAt": 300,
                    "messages": 3
                }),
                json!({
                    "id": "c1",
                    "startedAt": 100,
                    "endedAt": 200,
                    "title": "First",
                    "note": "did the thing",
                    "status": "done",
                    "messages": 2
                }),
            ]
        );
    }
}
