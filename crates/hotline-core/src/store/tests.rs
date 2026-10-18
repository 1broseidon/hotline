//! What the readers already answer, run through the contract's types and back
//! out.
//!
//! The tape, chapter, preview and search readers build their JSON key by
//! key, and they stay that way until a later task hands them those
//! structs. So the way to prove the structs are the same contract is to
//! take what those readers emit, parse it, write it again, and find
//! nothing changed. A field spelled differently there, or dropped, or
//! written as `null` where the reader wrote nothing at all, fails here.
//! The persona's proof is `room::roster`'s, because the room stream is
//! where a teammate is written down.

use crate::contract::{ChapterSummary, GlobalSearchHit, Preview, ThreadSearchHit, TranscriptEvent};
use crate::log::{Log, StreamId};
use crate::paths::transcript_segments_dir;
use crate::store::search::fixture::{chapter, index, message};
use crate::store::{chapters, previews, search};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> (PathBuf, Log) {
    let dir = std::env::temp_dir().join(format!(
        "hotline-core-contract-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("transcripts")).unwrap();
    let log = Log::open(&dir);
    (dir, log)
}

/// Parse `value` as `T` and write it back out. Anything but the value that
/// went in is a contract that has drifted from the reader.
fn round_trip<T>(value: &Value) -> Value
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let parsed: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("{value} does not read as this type: {error}"));
    serde_json::to_value(parsed).unwrap()
}

fn write_tape(root: &Path, persona_id: &str, events: &[Value]) {
    let dir = transcript_segments_dir(root, persona_id);
    std::fs::create_dir_all(&dir).unwrap();
    let lines: Vec<String> = events.iter().map(ToString::to_string).collect();
    std::fs::write(dir.join("1.jsonl"), lines.join("\n")).unwrap();
}

/// A tape holding one line of every kind, each carrying every optional
/// field it can — and, alongside them, the barest legal spelling of the
/// kinds whose optional fields are worth seeing omitted.
fn every_kind() -> Vec<Value> {
    vec![
        json!({
            "kind": "user", "id": "u1", "ts": 1, "text": "hi",
            "attachments": [{ "kind": "image", "name": "shot.png", "path": "/tmp/shot.png", "mimeType": "image/png", "size": 12 }],
            "reactions": ["🐸"], "replyTo": "a0",
            "scheduled": { "jobId": "j1", "kind": "loop", "name": "daily", "operatorCreated": false, "quiet": true },
            "ring": "attention", "receipt": "read",
        }),
        json!({ "kind": "user", "id": "u2", "ts": 2, "text": "bare" }),
        json!({
            "kind": "agent", "id": "a1", "ts": 3, "text": "hello",
            "reactions": ["👍"], "ring": "problem", "receipt": "sent",
        }),
        json!({ "kind": "thought", "id": "th1", "ts": 4, "text": "still thinking" }),
        json!({
            "kind": "tool", "id": "t1", "ts": 5, "toolCallId": "call-1", "title": "Edit",
            "toolKind": "edit", "status": "in_progress", "locations": ["/tmp/a"],
            "output": [
                { "type": "text", "text": "done" },
                { "type": "diff", "path": "/tmp/a", "oldText": null, "newText": "next" },
                { "type": "diff", "path": "/tmp/b", "oldText": "was", "newText": "next" },
            ],
        }),
        json!({ "kind": "tool", "id": "t2", "ts": 6, "toolCallId": "call-2", "title": "Read", "status": "completed" }),
        json!({
            "kind": "permission", "id": "p1", "ts": 7, "requestId": "r1", "title": "Write?",
            "options": [{ "optionId": "yes", "name": "Allow", "kind": "allow_once" }],
            "decision": "yes", "decidedOptionName": "Allow",
        }),
        json!({ "kind": "plan", "id": "pl1", "ts": 8, "entries": [{ "content": "ship it", "status": "pending", "priority": "high" }] }),
        json!({ "kind": "notice", "id": "n1", "ts": 9, "level": "warn", "text": "careful" }),
        json!({ "kind": "computer_frame", "id": "cf1", "ts": 10, "dataUrl": "data:image/png;base64,AA" }),
        json!({ "kind": "human_action", "id": "ha1", "ts": 11, "actionId": "act-1", "reason": "log in", "status": "pending" }),
        json!({
            "kind": "peer", "id": "pe1", "ts": 12, "threadKey": "ada|bob", "withPersonaId": "bob",
            "withName": "Boris", "role": "caller", "exchanges": 2, "status": "waiting", "seat": "client",
        }),
        json!({ "kind": "turn", "id": "tu1", "ts": 13, "stopReason": "end_turn", "usage": { "inputTokens": 1, "outputTokens": 2, "totalTokens": 3 } }),
        json!({
            "kind": "chapter", "id": "c1", "ts": 14, "backendId": "hotline", "sessionId": "s1",
            "endedAt": 20, "title": "First", "note": "did the thing", "status": "done",
            "tags": ["harbour"], "closedBy": "idle", "resumedFrom": "c0",
        }),
        json!({ "kind": "chapter", "id": "c2", "ts": 21, "backendId": "hotline" }),
    ]
}

#[test]
fn every_line_the_tape_holds_reads_back_as_the_transcript_event_it_was() {
    let (root, log) = scratch("contract-transcript");
    write_tape(&root, "ada", &every_kind());

    let loaded = log.load(&StreamId::Tape("ada".into()));
    assert_eq!(loaded.len(), every_kind().len());
    for event in &loaded {
        assert_eq!(&round_trip::<TranscriptEvent>(event), event);
    }
}

#[test]
fn the_drawers_chapters_and_the_rosters_preview_read_back_unchanged() {
    let (root, log) = scratch("contract-views");
    write_tape(&root, "ada", &every_kind());

    let listed = chapters::list(&log, "ada");
    assert_eq!(listed.len(), 2);
    for summary in &listed {
        assert_eq!(&round_trip::<ChapterSummary>(summary), summary);
    }

    let preview = previews::preview(&root, "ada").unwrap();
    assert_eq!(round_trip::<Preview>(&preview), preview);
}

#[test]
fn both_search_hits_read_back_as_the_hit_the_index_answered() {
    let (root, database) = index("contract-hits");
    message(&database, "ada", "m1", "user", "the harbour crane");
    message(&database, "bob", "m2", "agent", "the harbour again");
    chapter(
        &database,
        "bob",
        "c1",
        "Harbour week",
        "we fixed the harbour",
    );

    let thread = search::search(&root, "ada", "harbour", None).unwrap();
    let thread_hits = thread["hits"].as_array().unwrap();
    assert_eq!(thread_hits.len(), 1);
    for hit in thread_hits {
        assert_eq!(&round_trip::<ThreadSearchHit>(hit), hit);
    }

    let everyone = search::search_all(&root, "harbour", None).unwrap();
    let global_hits = everyone["hits"].as_array().unwrap();
    assert_eq!(global_hits.len(), 3);
    for hit in global_hits {
        assert_eq!(&round_trip::<GlobalSearchHit>(hit), hit);
    }
}
