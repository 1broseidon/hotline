//! A teammate's tape.
//!
//! Append-only JSONL, one segment per owner epoch, the legacy flat file
//! standing in for epoch 1. Some events are written more than once: a tool
//! call moves from pending to completed, a permission gets answered. Later
//! lines with the same `id` supersede earlier ones, so a load folds every
//! segment in epoch order and keeps each id where it first appeared, and a
//! compaction rewrites the open segment with that fold.
//!
//! Written to be byte-for-byte what `src/bun/store/transcript.ts` writes: a
//! tape this appends to is one the main reads unchanged, and the other way
//! round, because for the length of the migration both of them are looking at
//! the same files.

use crate::paths::{transcript_path, transcript_segment_path, transcript_segments_dir};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Every on-disk segment, oldest epoch first. The flat file counts as epoch
/// 1 only when `1.jsonl` is not already there.
pub(crate) fn segments_of(root: &Path, persona_id: &str) -> Vec<(u64, PathBuf)> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(transcript_segments_dir(root, persona_id)) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".jsonl")) else {
                continue;
            };
            let numeric = !stem.is_empty()
                && !stem.starts_with('0')
                && stem.bytes().all(|byte| byte.is_ascii_digit());
            if let (true, Ok(epoch)) = (numeric, stem.parse::<u64>()) {
                found.push((epoch, transcript_segment_path(root, persona_id, epoch)));
            }
        }
    }
    let flat = transcript_path(root, persona_id);
    if flat.exists() && !found.iter().any(|(epoch, _)| *epoch == 1) {
        found.push((1, flat));
    }
    found.sort_by_key(|(epoch, _)| *epoch);
    found
}

/// One line is one event; a torn final line from an unclean exit is skipped.
pub(crate) fn parse_lines(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
}

pub(crate) fn fold(events: impl Iterator<Item = Value>) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for event in events {
        let Some(id) = event.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if !by_id.contains_key(&id) {
            order.push(id.clone());
        }
        by_id.insert(id, event);
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

/// The whole tape, folded. An unknown teammate has an empty one.
pub fn load(root: &Path, persona_id: &str) -> Vec<Value> {
    let mut events = Vec::new();
    for (_, path) in segments_of(root, persona_id) {
        if let Ok(text) = fs::read_to_string(&path) {
            events.extend(parse_lines(&text));
        }
    }
    fold(events.into_iter())
}

/// One local write to the open epoch, as replication sees it: which bytes
/// landed at which offset. The bytes are the serialized line, newline included.
///
/// `append` hands this back instead of ringing a seam the way the main does.
/// The tape must not know about wires either way, and a return value is the
/// version with one fewer moving part: the caller that has a mesh to feed
/// pushes it, and the caller that has none drops it on the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptAppend {
    pub persona_id: String,
    pub epoch: u64,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// The epoch a teammate's tape is open at: the highest segment on disk, or 1
/// for a tape that has none yet.
///
/// The previous Toad took this from the teammate's record, because a hop
/// between desks bumped it there first and the tape followed. Here there is
/// no record store and no hop yet; the segments already on disk — some of
/// them written by that Toad, past a hop — are the whole truth of where the
/// tape is up to. When moving a teammate returns, it is the thing that will
/// open the next segment, and this is where it will say so.
pub(crate) fn open_epoch(root: &Path, persona_id: &str) -> u64 {
    segments_of(root, persona_id)
        .last()
        .map(|(epoch, _)| *epoch)
        .unwrap_or(1)
}

/// Relocates the legacy flat file if needed, then answers the segment this
/// process may write.
///
/// Refuses when both the flat file and `1.jsonl` exist. Readers are allowed to
/// guess which of the two is the tape; a writer that guessed wrong would fork
/// the history, so it stops instead.
fn writable_segment(root: &Path, persona_id: &str) -> io::Result<(PathBuf, u64)> {
    let flat = transcript_path(root, persona_id);
    let epoch_one = transcript_segment_path(root, persona_id, 1);
    let segments = transcript_segments_dir(root, persona_id);
    if flat.exists() && epoch_one.exists() {
        return Err(io::Error::other(format!(
            "Refusing to write transcript for {persona_id}: both {} and {} exist.",
            flat.display(),
            epoch_one.display()
        )));
    }
    fs::create_dir_all(&segments)?;
    if flat.exists() {
        fs::rename(&flat, &epoch_one)?;
    }
    let epoch = open_epoch(root, persona_id);
    Ok((transcript_segment_path(root, persona_id, epoch), epoch))
}

/// Adds one event to the end of the open segment.
pub fn append(root: &Path, persona_id: &str, event: &Value) -> io::Result<TranscriptAppend> {
    let (path, epoch) = writable_segment(root, persona_id)?;
    let offset = fs::metadata(&path).map_or(0, |file| file.len());
    let bytes = format!("{event}\n").into_bytes();
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&bytes)?;
    Ok(TranscriptAppend {
        persona_id: persona_id.to_string(),
        epoch,
        offset,
        bytes,
    })
}

/// Rewrites the current-epoch segment with folded history, and answers the
/// epoch it rewrote. Older segments stay put.
///
/// A fold that changes nothing skips the write and answers `None`, because a
/// caller that announces a rewrite costs every mirror its copy of the epoch,
/// and a rewrite nobody made is not worth that.
pub fn compact(root: &Path, persona_id: &str) -> io::Result<Option<u64>> {
    let (file, epoch) = writable_segment(root, persona_id)?;
    if !file.exists() {
        return Ok(None);
    }
    let before = fs::read_to_string(&file)?;
    let events = fold(parse_lines(&before));
    if events.is_empty() {
        return Ok(None);
    }
    let after = events
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join("\n")
        + "\n";
    if after == before {
        return Ok(None);
    }
    fs::write(&file, after)?;
    Ok(Some(epoch))
}

/// The permission cards a restart orphaned, superseded as expired.
///
/// A resolver only exists in the process that received the ACP request, so a
/// card that still claims to be live after a restart is a button nobody is
/// behind. This is `expireOrphanedPermissions` in `src/bun/acp/permissions.ts`;
/// it lives here because peer threads carry the same cards and there should be
/// one copy of the rule. The caller appends what it answers and then compacts,
/// which is the whole startup fold — the same three steps for a tape as for a
/// thread, spelled at the caller in `src/bun/index.ts` and spelled there here.
///
/// A card with a `decision` of `null` is left alone: the main tests it against
/// `undefined`, and a decision somebody wrote is a decision.
pub fn expire_orphaned_permissions(events: &[Value], ts: i64) -> Vec<Value> {
    events
        .iter()
        .filter(|event| {
            event.get("kind").and_then(Value::as_str) == Some("permission")
                && event.get("decision").is_none()
        })
        .filter_map(|event| {
            let mut expired = event.as_object()?.clone();
            expired.insert("ts".into(), Value::from(ts));
            expired.insert("decision".into(), Value::from("expired"));
            Some(Value::Object(expired))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "toad-core-transcript-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("transcripts")).unwrap();
        dir
    }

    #[test]
    fn an_unknown_teammate_has_an_empty_tape() {
        let root = scratch("empty");
        assert!(load(&root, "nobody").is_empty());
    }

    #[test]
    fn segments_fold_in_epoch_order_and_later_lines_win_by_id() {
        let root = scratch("fold");
        let dir = transcript_segments_dir(&root, "p");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("1.jsonl"),
            [
                json!({"kind": "user", "id": "u1", "ts": 1, "text": "hi"}).to_string(),
                json!({"kind": "tool", "id": "t1", "ts": 2, "status": "pending"}).to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        fs::write(
            dir.join("2.jsonl"),
            format!(
                "{}\n{}\n{{\"kind\":\"agent\",\"id\":\"torn",
                json!({"kind": "tool", "id": "t1", "ts": 2, "status": "done"}),
                json!({"kind": "agent", "id": "a1", "ts": 3, "text": "hello"}),
            ),
        )
        .unwrap();

        let events = load(&root, "p");
        let ids: Vec<&str> = events.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["u1", "t1", "a1"]);
        assert_eq!(events[1]["status"], "done");
    }

    #[test]
    fn the_flat_file_is_epoch_one_unless_a_real_epoch_one_exists() {
        let root = scratch("flat");
        fs::write(
            transcript_path(&root, "p"),
            json!({"kind": "user", "id": "flat", "ts": 1, "text": "old"}).to_string(),
        )
        .unwrap();
        let dir = transcript_segments_dir(&root, "p");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("2.jsonl"),
            json!({"kind": "user", "id": "later", "ts": 2, "text": "new"}).to_string(),
        )
        .unwrap();
        let ids: Vec<String> = load(&root, "p")
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["flat", "later"]);

        fs::write(
            dir.join("1.jsonl"),
            json!({"kind": "user", "id": "real", "ts": 1, "text": "x"}).to_string(),
        )
        .unwrap();
        let ids: Vec<String> = load(&root, "p")
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["real", "later"]);
    }

    fn user(id: &str, ts: i64, text: &str) -> Value {
        json!({"kind": "user", "id": id, "ts": ts, "text": text})
    }

    fn tool(id: &str, ts: i64, status: &str) -> Value {
        json!({"kind": "tool", "id": id, "ts": ts, "toolCallId": id, "title": "run", "status": status})
    }

    fn permission(id: &str, ts: i64) -> Value {
        json!({
            "kind": "permission", "id": id, "ts": ts, "requestId": format!("req-{id}"),
            "title": "read a file",
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}]
        })
    }

    fn write_segment(root: &Path, persona_id: &str, epoch: u64, events: &[Value]) {
        let path = transcript_segment_path(root, persona_id, epoch);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lines: Vec<String> = events.iter().map(ToString::to_string).collect();
        fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn segment(root: &Path, persona_id: &str, epoch: u64) -> String {
        fs::read_to_string(transcript_segment_path(root, persona_id, epoch)).unwrap()
    }

    #[test]
    fn reading_a_legacy_flat_file_does_not_move_it() {
        let root = scratch("read-flat");
        let line = format!("{}\n", user("u1", 1, "from the flat file"));
        fs::write(transcript_path(&root, "p"), &line).unwrap();

        assert_eq!(load(&root, "p"), [user("u1", 1, "from the flat file")]);
        assert!(transcript_path(&root, "p").exists());
        assert!(!transcript_segments_dir(&root, "p").exists());
    }

    #[test]
    fn the_first_write_relocates_the_flat_file_by_rename_and_keeps_its_bytes() {
        let root = scratch("relocate");
        let original = format!("{}\n", user("u1", 1, "keep these bytes"));
        fs::write(transcript_path(&root, "p"), &original).unwrap();

        let added = user("u2", 2, "after the move");
        append(&root, "p", &added).unwrap();

        assert!(!transcript_path(&root, "p").exists());
        let moved = segment(&root, "p", 1);
        assert_eq!(&moved[..original.len()], original);
        assert_eq!(&moved[original.len()..], format!("{added}\n"));
    }

    #[test]
    fn a_first_append_lands_in_epoch_one_and_says_which_bytes_landed_where() {
        let root = scratch("append");
        let first = append(&root, "p", &user("u1", 1, "first write")).unwrap();

        assert!(!transcript_path(&root, "p").exists());
        assert_eq!(
            segment(&root, "p", 1),
            format!("{}\n", user("u1", 1, "first write"))
        );
        assert_eq!(
            first,
            TranscriptAppend {
                persona_id: "p".into(),
                epoch: 1,
                offset: 0,
                bytes: format!("{}\n", user("u1", 1, "first write")).into_bytes(),
            }
        );

        // The next write starts where the last one ended: replication answers
        // "what am I missing" by subtracting offsets, so they have to add up.
        let second = append(&root, "p", &user("u2", 2, "second write")).unwrap();
        assert_eq!(second.offset, first.bytes.len() as u64);
        assert_eq!(
            second.offset + second.bytes.len() as u64,
            fs::metadata(transcript_segment_path(&root, "p", 1))
                .unwrap()
                .len()
        );
    }

    /// The open segment is the highest epoch on disk; older ones are closed
    /// history that a compaction never touches, duplicates and all.
    #[test]
    fn compact_touches_only_the_open_segment() {
        let root = scratch("compact");
        let earlier = format!(
            "{}\n{}\n",
            tool("t0", 1, "pending"),
            tool("t0", 2, "completed")
        );
        fs::create_dir_all(transcript_segments_dir(&root, "p")).unwrap();
        fs::write(transcript_segment_path(&root, "p", 1), &earlier).unwrap();
        write_segment(
            &root,
            "p",
            2,
            &[
                tool("t1", 3, "pending"),
                tool("t1", 4, "completed"),
                user("u1", 5, "in epoch 2"),
            ],
        );

        assert_eq!(compact(&root, "p").unwrap(), Some(2));

        assert_eq!(segment(&root, "p", 1), earlier);
        let ids: Vec<String> = load(&root, "p")
            .iter()
            .map(|event| event["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["t0", "t1", "u1"]);
        assert_eq!(
            segment(&root, "p", 2),
            format!(
                "{}\n{}\n",
                tool("t1", 4, "completed"),
                user("u1", 5, "in epoch 2")
            )
        );
    }

    #[test]
    fn a_real_rewrite_names_its_epoch_and_a_no_op_compact_touches_nothing() {
        let root = scratch("no-op");
        write_segment(
            &root,
            "p",
            1,
            &[tool("t1", 1, "pending"), tool("t1", 2, "completed")],
        );

        assert_eq!(compact(&root, "p").unwrap(), Some(1));
        let folded = segment(&root, "p", 1);

        // Already folded: no write, and no epoch named, because announcing one
        // costs every mirror its copy of this epoch.
        assert_eq!(compact(&root, "p").unwrap(), None);
        assert_eq!(segment(&root, "p", 1), folded);

        // Nothing to fold at all is the same answer, and creates no file.
        assert_eq!(compact(&root, "nobody").unwrap(), None);
        assert!(!transcript_segment_path(&root, "nobody", 1).exists());
    }

    #[test]
    fn a_writer_refuses_a_tape_that_is_both_a_flat_file_and_an_epoch_one() {
        let root = scratch("both");
        let flat = format!("{}\n", user("flat", 1, "legacy"));
        let epoch_one = format!("{}\n", user("seg", 1, "segment"));
        fs::write(transcript_path(&root, "p"), &flat).unwrap();
        write_segment(&root, "p", 1, &[user("seg", 1, "segment")]);

        let refusal = append(&root, "p", &user("nope", 2, "should not land")).unwrap_err();
        assert!(refusal.to_string().contains("both"));
        assert!(
            compact(&root, "p")
                .unwrap_err()
                .to_string()
                .contains("both")
        );
        assert_eq!(
            fs::read_to_string(transcript_path(&root, "p")).unwrap(),
            flat
        );
        assert_eq!(segment(&root, "p", 1), epoch_one);
    }

    #[test]
    fn the_startup_fold_expires_the_cards_the_restart_orphaned() {
        // The sequence in `src/bun/index.ts`: for every teammate, expire the
        // orphaned cards, append the expiries, then compact.
        let root = scratch("startup");
        for event in [
            user("u1", 1000, "hello"),
            permission("p1", 1002),
            permission("p2", 1003),
        ] {
            append(&root, "p", &event).unwrap();
        }
        // One card was answered before the crash and must be left alone.
        let mut answered = permission("p2", 1003);
        answered["decision"] = Value::from("allow");
        append(&root, "p", &answered).unwrap();

        for expired in expire_orphaned_permissions(&load(&root, "p"), 2000) {
            append(&root, "p", &expired).unwrap();
        }
        compact(&root, "p").unwrap();

        let events = load(&root, "p");
        assert_eq!(events.len(), 3);
        assert_eq!(events[1]["decision"], "expired");
        assert_eq!(events[1]["ts"], 2000);
        assert_eq!(events[2]["decision"], "allow");
        // Once answered, a second startup finds nothing left to expire.
        assert!(expire_orphaned_permissions(&events, 3000).is_empty());
    }

    /// The bytes below came out of the main's own writer. Produced by running,
    /// against a throwaway `TOAD_DATA_DIR`, a script that calls
    /// `src/bun/store/transcript.ts`'s `append` with these five events, then
    /// `expireOrphanedPermissions(transcript.load(id), 2000)`, appends what it
    /// answers, calls `compact`, and prints the segment file.
    #[test]
    fn a_tape_this_writes_is_byte_for_byte_the_one_the_main_writes() {
        let root = scratch("bytes");
        for event in [
            user("u1", 1000, "hello"),
            tool("t1", 1001, "pending"),
            permission("p1", 1002),
            tool("t1", 1003, "completed"),
            json!({"kind": "agent", "id": "a1", "ts": 1004, "text": "done"}),
        ] {
            append(&root, "pin-persona", &event).unwrap();
        }
        assert_eq!(
            segment(&root, "pin-persona", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1001,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"pending\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":1002,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}]}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"agent\",\"id\":\"a1\",\"ts\":1004,\"text\":\"done\"}\n"
        );

        for expired in expire_orphaned_permissions(&load(&root, "pin-persona"), 2000) {
            append(&root, "pin-persona", &expired).unwrap();
        }
        assert_eq!(
            segment(&root, "pin-persona", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1001,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"pending\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":1002,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}]}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"agent\",\"id\":\"a1\",\"ts\":1004,\"text\":\"done\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":2000,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}],\"decision\":\"expired\"}\n"
        );

        compact(&root, "pin-persona").unwrap();
        assert_eq!(
            segment(&root, "pin-persona", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":2000,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}],\"decision\":\"expired\"}\n{\"kind\":\"agent\",\"id\":\"a1\",\"ts\":1004,\"text\":\"done\"}\n"
        );
    }
}
