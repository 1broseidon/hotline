//! The last thing either side said, for the roster to show under a name.
//!
//! Reads the end of a tape rather than the whole of it: this runs for every
//! local teammate at startup, and a transcript is only bounded by how much
//! has been said. The tail is read raw, not folded — a chapter marker or a
//! tool call mutates in place by id, but a message never does, so the id
//! folding `Log::load` does for the rest of the tape buys nothing here and
//! costs a second full read. That is why this reads the tape's segments
//! itself instead of asking the log for the fold.

use crate::log::{parse_lines, segments_of};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How far back to look for the last thing said. A message is the last line
/// in a transcript far more often than not, and a teammate that has spent
/// the last 64KB on tool calls alone has nothing worth previewing anyway.
const TAIL_BYTES: u64 = 64 * 1024;

fn segment_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn logical_size(segments: &[(u64, PathBuf)]) -> u64 {
    segments.iter().map(|(_, path)| segment_size(path)).sum()
}

/// The last `window` bytes of the logical tape — segments concatenated in
/// epoch order. Starts at the end of the highest-epoch non-empty segment and
/// walks into earlier ones only while the window still needs bytes.
fn read_tail_logical(segments: &[(u64, PathBuf)], window: u64) -> Vec<Value> {
    let sizes: Vec<u64> = segments
        .iter()
        .map(|(_, path)| segment_size(path))
        .collect();
    let total: u64 = sizes.iter().sum();
    if total == 0 || window == 0 {
        return Vec::new();
    }
    let length = window.min(total);

    let mut remaining = length;
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut started_mid = false;
    for index in (0..segments.len()).rev() {
        if remaining == 0 {
            break;
        }
        let size = sizes[index];
        if size == 0 {
            continue;
        }
        let take = size.min(remaining);
        let offset = size - take;
        let mut buffer = vec![0u8; take as usize];
        if let Ok(mut file) = File::open(&segments[index].1)
            && file.seek(SeekFrom::Start(offset)).is_ok()
        {
            let _ = file.read_exact(&mut buffer);
        }
        chunks.push(buffer);
        // The last segment we touch is the oldest in the window. A read that
        // does not start at that file's byte 0 can land mid-line.
        started_mid = offset > 0;
        remaining -= take;
    }
    chunks.reverse();
    let bytes: Vec<u8> = chunks.into_iter().flatten().collect();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let sliced: &str = if started_mid {
        text.find('\n')
            .map(|index| &text[index + 1..])
            .unwrap_or("")
    } else {
        &text
    };
    parse_lines(sliced).collect()
}

fn message_preview(event: &Value) -> Option<Value> {
    let from = match event.get("kind").and_then(Value::as_str)? {
        "user" => "me",
        "agent" => "them",
        _ => return None,
    };
    let text = event.get("text").and_then(Value::as_str).unwrap_or("");
    let at = event.get("ts").cloned().unwrap_or(Value::Null);
    Some(json!({ "from": from, "text": text, "at": at }))
}

/// The last stretch of a teammate's tape, raw and unfolded: the events in
/// the tail window, oldest first. What the preview and the roster's running
/// tool are both read from, because both are questions about the end of the
/// tape and a whole-tape fold answers them at the cost of the whole tape.
pub fn tail(root: &Path, persona_id: &str) -> Vec<Value> {
    let segments = segments_of(root, persona_id);
    if logical_size(&segments) == 0 {
        return Vec::new();
    }
    read_tail_logical(&segments, TAIL_BYTES)
}

/// The last thing either side said in one teammate's tape, or nothing for a
/// teammate that has never spoken.
pub fn preview(root: &Path, persona_id: &str) -> Option<Value> {
    tail(root, persona_id)
        .iter()
        .rev()
        .find_map(message_preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::transcript_segments_dir;
    use serde_json::json;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hotline-core-previews-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("transcripts")).unwrap();
        dir
    }

    fn write_tape(root: &Path, persona_id: &str, epoch: u64, events: &[Value]) {
        let dir = transcript_segments_dir(root, persona_id);
        fs::create_dir_all(&dir).unwrap();
        let lines: Vec<String> = events.iter().map(ToString::to_string).collect();
        fs::write(
            dir.join(format!("{epoch}.jsonl")),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();
    }

    #[test]
    fn a_teammate_with_no_tape_has_no_preview() {
        let root = scratch("previews-none");
        assert!(preview(&root, "nobody").is_none());
    }

    #[test]
    fn the_last_user_line_previews_as_me() {
        let root = scratch("previews-user");
        write_tape(
            &root,
            "p",
            1,
            &[
                json!({"kind": "agent", "id": "a1", "ts": 1, "text": "hello"}),
                json!({"kind": "user", "id": "u1", "ts": 2, "text": "hi back"}),
            ],
        );
        assert_eq!(
            preview(&root, "p"),
            Some(json!({"from": "me", "text": "hi back", "at": 2}))
        );
    }

    #[test]
    fn the_last_agent_line_previews_as_them() {
        let root = scratch("previews-agent");
        write_tape(
            &root,
            "p",
            1,
            &[
                json!({"kind": "user", "id": "u1", "ts": 1, "text": "hi"}),
                json!({"kind": "agent", "id": "a1", "ts": 2, "text": "hello there"}),
            ],
        );
        assert_eq!(
            preview(&root, "p"),
            Some(json!({"from": "them", "text": "hello there", "at": 2}))
        );
    }

    #[test]
    fn tool_and_chapter_events_after_the_last_message_are_ignored() {
        let root = scratch("previews-trailing");
        write_tape(
            &root,
            "p",
            1,
            &[
                json!({"kind": "user", "id": "u1", "ts": 1, "text": "start"}),
                json!({"kind": "agent", "id": "a1", "ts": 2, "text": "the last word"}),
                json!({"kind": "tool", "id": "t1", "ts": 3, "status": "pending"}),
                json!({"kind": "chapter", "id": "c1", "ts": 4, "status": "in-progress"}),
            ],
        );
        assert_eq!(
            preview(&root, "p"),
            Some(json!({"from": "them", "text": "the last word", "at": 2}))
        );
    }

    /// A tape well past the 64 KiB tail window, where the boundary lands
    /// inside an earlier line rather than on one of its ends: the partial
    /// line that falls off the front of the window is dropped, and the real
    /// last message just past it still reads whole.
    #[test]
    fn a_line_the_tail_window_cuts_in_half_is_dropped_not_misread() {
        let root = scratch("previews-tail-cut");
        let padding = json!({"kind": "tool", "id": "t1", "ts": 1, "status": "pending", "note": "x".repeat(80_000)});
        write_tape(
            &root,
            "p",
            1,
            &[
                padding,
                json!({"kind": "user", "id": "u2", "ts": 2, "text": "tail hit"}),
            ],
        );
        let size = segment_size(&transcript_segments_dir(&root, "p").join("1.jsonl"));
        assert!(
            size > TAIL_BYTES,
            "the tape must exceed the tail window to exercise it"
        );
        assert_eq!(
            preview(&root, "p"),
            Some(json!({"from": "me", "text": "tail hit", "at": 2}))
        );
    }

    /// Mirrors Bun's "quiet tail" case: the newest segment has nothing but a
    /// non-message event, so the walk must cross into the older segment to
    /// find the last thing either side said.
    #[test]
    fn a_preview_spanning_two_segments_finds_the_message_in_the_older_one() {
        let root = scratch("previews-span");
        write_tape(
            &root,
            "p",
            1,
            &[json!({"kind": "user", "id": "u1", "ts": 1, "text": "hello"})],
        );
        write_tape(
            &root,
            "p",
            2,
            &[json!({"kind": "thought", "id": "th1", "ts": 2, "text": "still thinking"})],
        );
        assert_eq!(
            preview(&root, "p"),
            Some(json!({"from": "me", "text": "hello", "at": 1}))
        );
    }
}
