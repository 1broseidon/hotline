//! Where a teammate's tape lives on disk.
//!
//! One segment per owner epoch, `transcripts/<id>/<epoch>.jsonl`, with the
//! legacy flat file `transcripts/<id>.jsonl` standing in for epoch 1. The open
//! segment — the one an append lands in — is the highest epoch on disk.
//!
//! This layout is the previous Toad's, spelled the same way on purpose: a tape
//! this writes is one that Toad reads unchanged, so importing a data directory
//! copies its tapes and changes nothing about them.

use crate::paths::{transcript_path, transcript_segment_path, transcript_segments_dir};
use std::fs;
use std::io;
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

/// The epoch a teammate's tape is open at: the highest segment on disk, or 1
/// for a tape that has none yet.
///
/// The previous Toad took this from the teammate's record, because a hop
/// between desks bumped it there first and the tape followed. Here there is
/// no record store and no hop, so the segments already on disk — some of them
/// written by that Toad, past a hop — are the whole truth of where the tape is
/// up to. When moving a teammate returns, it is the thing that will open the
/// next segment, and this is where it will say so.
pub(crate) fn open_epoch(root: &Path, persona_id: &str) -> u64 {
    segments_of(root, persona_id)
        .last()
        .map(|(epoch, _)| *epoch)
        .unwrap_or(1)
}

/// Relocates the legacy flat file if needed, then answers the segment this
/// process may write and which epoch it is.
///
/// Refuses when both the flat file and `1.jsonl` exist. Readers are allowed to
/// guess which of the two is the tape; a writer that guessed wrong would fork
/// the history, so it stops instead.
pub(crate) fn writable_segment(root: &Path, persona_id: &str) -> io::Result<(PathBuf, u64)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{Appended, Log, StreamId, expire_orphaned_permissions};
    use serde_json::{Value, json};

    fn scratch(name: &str) -> (PathBuf, Log) {
        let root =
            std::env::temp_dir().join(format!("toad-core-tape-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("transcripts")).unwrap();
        let log = Log::open(&root);
        (root, log)
    }

    fn stream(persona_id: &str) -> StreamId {
        StreamId::Tape(persona_id.to_string())
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
    fn an_unknown_teammate_has_an_empty_tape() {
        let (_root, log) = scratch("empty");
        assert!(log.load(&stream("nobody")).is_empty());
    }

    #[test]
    fn segments_fold_in_epoch_order_and_later_lines_win_by_id() {
        let (root, log) = scratch("fold");
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

        let events = log.load(&stream("p"));
        let ids: Vec<&str> = events.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["u1", "t1", "a1"]);
        assert_eq!(events[1]["status"], "done");
    }

    #[test]
    fn the_flat_file_is_epoch_one_unless_a_real_epoch_one_exists() {
        let (root, log) = scratch("flat");
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
        let ids: Vec<String> = log
            .load(&stream("p"))
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["flat", "later"]);

        fs::write(
            dir.join("1.jsonl"),
            json!({"kind": "user", "id": "real", "ts": 1, "text": "x"}).to_string(),
        )
        .unwrap();
        let ids: Vec<String> = log
            .load(&stream("p"))
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["real", "later"]);
    }

    #[test]
    fn reading_a_legacy_flat_file_does_not_move_it() {
        let (root, log) = scratch("read-flat");
        let line = format!("{}\n", user("u1", 1, "from the flat file"));
        fs::write(transcript_path(&root, "p"), &line).unwrap();

        assert_eq!(
            log.load(&stream("p")),
            [user("u1", 1, "from the flat file")]
        );
        assert!(transcript_path(&root, "p").exists());
        assert!(!transcript_segments_dir(&root, "p").exists());
    }

    #[test]
    fn the_first_write_relocates_the_flat_file_by_rename_and_keeps_its_bytes() {
        let (root, log) = scratch("relocate");
        let original = format!("{}\n", user("u1", 1, "keep these bytes"));
        fs::write(transcript_path(&root, "p"), &original).unwrap();

        let added = user("u2", 2, "after the move");
        log.append(&stream("p"), &added).unwrap();

        assert!(!transcript_path(&root, "p").exists());
        let moved = segment(&root, "p", 1);
        assert_eq!(&moved[..original.len()], original);
        assert_eq!(&moved[original.len()..], format!("{added}\n"));
    }

    #[test]
    fn a_first_append_lands_in_epoch_one_and_says_which_bytes_landed_where() {
        let (root, log) = scratch("append");
        let first = log
            .append(&stream("p"), &user("u1", 1, "first write"))
            .unwrap();

        assert!(!transcript_path(&root, "p").exists());
        assert_eq!(
            segment(&root, "p", 1),
            format!("{}\n", user("u1", 1, "first write"))
        );
        assert_eq!(
            first,
            Appended {
                epoch: 1,
                offset: 0,
                bytes: format!("{}\n", user("u1", 1, "first write")).into_bytes(),
            }
        );

        // The next write starts where the last one ended: replication answers
        // "what am I missing" by subtracting offsets, so they have to add up.
        let second = log
            .append(&stream("p"), &user("u2", 2, "second write"))
            .unwrap();
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
        let (root, log) = scratch("compact");
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

        assert_eq!(log.compact(&stream("p")).unwrap(), Some(2));

        assert_eq!(segment(&root, "p", 1), earlier);
        let ids: Vec<String> = log
            .load(&stream("p"))
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
        let (root, log) = scratch("no-op");
        write_segment(
            &root,
            "p",
            1,
            &[tool("t1", 1, "pending"), tool("t1", 2, "completed")],
        );

        assert_eq!(log.compact(&stream("p")).unwrap(), Some(1));
        let folded = segment(&root, "p", 1);

        // Already folded: no write, and no epoch named, because announcing one
        // costs every mirror its copy of this epoch.
        assert_eq!(log.compact(&stream("p")).unwrap(), None);
        assert_eq!(segment(&root, "p", 1), folded);

        // Nothing to fold at all is the same answer, and creates no file.
        assert_eq!(log.compact(&stream("nobody")).unwrap(), None);
        assert!(!transcript_segment_path(&root, "nobody", 1).exists());
    }

    #[test]
    fn a_writer_refuses_a_tape_that_is_both_a_flat_file_and_an_epoch_one() {
        let (root, log) = scratch("both");
        let flat = format!("{}\n", user("flat", 1, "legacy"));
        let epoch_one = format!("{}\n", user("seg", 1, "segment"));
        fs::write(transcript_path(&root, "p"), &flat).unwrap();
        write_segment(&root, "p", 1, &[user("seg", 1, "segment")]);

        let refusal = log
            .append(&stream("p"), &user("nope", 2, "should not land"))
            .unwrap_err();
        assert!(refusal.to_string().contains("both"));
        assert!(
            log.compact(&stream("p"))
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
        // The sequence at startup: for every teammate, expire the orphaned
        // cards, append the expiries, then compact.
        let (_root, log) = scratch("startup");
        for event in [
            user("u1", 1000, "hello"),
            permission("p1", 1002),
            permission("p2", 1003),
        ] {
            log.append(&stream("p"), &event).unwrap();
        }
        // One card was answered before the crash and must be left alone.
        let mut answered = permission("p2", 1003);
        answered["decision"] = Value::from("allow");
        log.append(&stream("p"), &answered).unwrap();

        for expired in expire_orphaned_permissions(&log.load(&stream("p")), 2000) {
            log.append(&stream("p"), &expired).unwrap();
        }
        assert_eq!(log.compact(&stream("p")).unwrap(), Some(1));

        let events = log.load(&stream("p"));
        assert_eq!(events.len(), 3);
        assert_eq!(events[1]["decision"], "expired");
        assert_eq!(events[1]["ts"], 2000);
        assert_eq!(events[2]["decision"], "allow");
        // Once answered, a second startup finds nothing left to expire.
        assert!(expire_orphaned_permissions(&events, 3000).is_empty());
    }

    /// The bytes below came out of the previous Toad's own writer. Produced by
    /// running, against a throwaway `TOAD_DATA_DIR`, a script that calls
    /// `src/bun/store/transcript.ts`'s `append` with these five events, then
    /// `expireOrphanedPermissions(transcript.load(id), 2000)`, appends what it
    /// answers, calls `compact`, and prints the segment file.
    #[test]
    fn a_tape_this_writes_is_byte_for_byte_the_one_the_previous_toad_writes() {
        let (root, log) = scratch("bytes");
        for event in [
            user("u1", 1000, "hello"),
            tool("t1", 1001, "pending"),
            permission("p1", 1002),
            tool("t1", 1003, "completed"),
            json!({"kind": "agent", "id": "a1", "ts": 1004, "text": "done"}),
        ] {
            log.append(&stream("pin-persona"), &event).unwrap();
        }
        assert_eq!(
            segment(&root, "pin-persona", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1001,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"pending\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":1002,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}]}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"agent\",\"id\":\"a1\",\"ts\":1004,\"text\":\"done\"}\n"
        );

        for expired in expire_orphaned_permissions(&log.load(&stream("pin-persona")), 2000) {
            log.append(&stream("pin-persona"), &expired).unwrap();
        }
        assert_eq!(
            segment(&root, "pin-persona", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1001,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"pending\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":1002,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}]}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"agent\",\"id\":\"a1\",\"ts\":1004,\"text\":\"done\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":2000,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}],\"decision\":\"expired\"}\n"
        );

        assert_eq!(log.compact(&stream("pin-persona")).unwrap(), Some(1));
        assert_eq!(
            segment(&root, "pin-persona", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":2000,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}],\"decision\":\"expired\"}\n{\"kind\":\"agent\",\"id\":\"a1\",\"ts\":1004,\"text\":\"done\"}\n"
        );
    }
}
