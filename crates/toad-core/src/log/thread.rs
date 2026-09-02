//! Where one pair of teammates' conversation lives, and the sidecar beside it.
//!
//! A thread is a stream like any other, minus the epochs: nobody replicates
//! it, so there is one file per pair — `threads/<key>.jsonl` — and no segments
//! to keep apart. Beside it sits a small JSON sidecar naming the two
//! participants, which sessions they have resumed, and what to call a side the
//! roster cannot resolve. The sidecar is not events: it is a record of who is
//! in the room, rewritten in place.
//!
//! Written to be byte-for-byte what the previous Toad's `store/threads.ts`
//! writes, so a data directory imports with its threads unchanged.

use crate::paths::{decode_file_component, thread_meta_path, thread_path, threads_dir};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::Path;

fn missing_key(key: &str) -> io::Error {
    io::Error::other(format!("Invalid thread key: {key}"))
}

/// The file a thread's events are written to.
///
/// A key that does not spell itself the same way again — unsorted, three
/// sided, or climbing out of the directory — names no file, so it is refused
/// rather than written somewhere surprising.
pub(crate) fn file(root: &Path, key: &str) -> io::Result<std::path::PathBuf> {
    thread_path(root, key).ok_or_else(|| missing_key(key))
}

/// The sidecar, or nothing when it is absent, unreadable, or of another
/// version.
///
/// Read as free-form JSON rather than into a struct on purpose: the previous
/// Toad parses it with a cast, which keeps every field the file had, so a
/// writer that dropped the fields it does not know about would quietly delete
/// a newer build's work on the next `set_label`.
fn read_meta(root: &Path, key: &str) -> Option<Map<String, Value>> {
    let file = thread_meta_path(root, key)?;
    let meta: Value = serde_json::from_str(&fs::read_to_string(&file).ok()?).ok()?;
    let meta = meta.as_object()?;
    (meta.get("version").and_then(Value::as_f64) == Some(1.0)).then(|| meta.clone())
}

/// Replaces the sidecar through a temporary file, so a reader never catches a
/// half-written one. The pid in the name is what keeps two Toads from picking
/// the same temporary.
fn write_meta(root: &Path, key: &str, meta: &Map<String, Value>) -> io::Result<()> {
    let file = thread_meta_path(root, key).ok_or_else(|| missing_key(key))?;
    fs::create_dir_all(threads_dir(root))?;
    let mut temporary = file.clone().into_os_string();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = std::path::PathBuf::from(temporary);
    fs::write(
        &temporary,
        format!("{}\n", serde_json::to_string_pretty(meta)?),
    )?;
    fs::rename(&temporary, &file)
}

/// Records a display name for a side the roster cannot resolve.
///
/// A thread whose sidecar is missing is left alone rather than given one: the
/// sidecar is written when the conversation is opened, and inventing one here
/// would name participants this function was never told.
pub fn set_label(root: &Path, key: &str, side_id: &str, label: &str) -> io::Result<()> {
    let Some(mut meta) = read_meta(root, key) else {
        return Ok(());
    };
    let labels = meta.get("labels").and_then(Value::as_object);
    if labels.and_then(|labels| labels.get(side_id)) == Some(&Value::from(label)) {
        return Ok(());
    }
    let mut updated = labels.cloned().unwrap_or_default();
    updated.insert(side_id.to_string(), Value::from(label));
    meta.insert("labels".into(), Value::Object(updated));
    write_meta(root, key, &meta)
}

/// Every thread this desk holds a sidecar for.
///
/// The sidecar and not the stream, because a conversation exists from the
/// moment it is opened, whether or not anybody has said anything yet. A name
/// that does not decode back into a well-formed key is skipped: it is not a
/// thread, and asking for its path would be asking for a file outside the
/// directory.
pub fn list_all_keys(root: &Path) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let Ok(entries) = fs::read_dir(threads_dir(root)) else {
        return keys;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
            continue;
        };
        let key = decode_file_component(stem);
        if thread_meta_path(root, &key).is_some() && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{Log, StreamId, expire_orphaned_permissions};
    use serde_json::json;
    use std::path::PathBuf;

    fn scratch(name: &str) -> (PathBuf, Log) {
        let root =
            std::env::temp_dir().join(format!("toad-core-thread-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(threads_dir(&root)).unwrap();
        let log = Log::open(&root);
        (root, log)
    }

    fn stream(key: &str) -> StreamId {
        StreamId::Thread(key.to_string())
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

    fn meta(key: &str) -> Map<String, Value> {
        let (a, b) = key.split_once('~').unwrap();
        json!({
            "version": 1, "a": a, "b": b,
            "sides": {"user": a, "agent": b},
            "sessions": [],
            "createdAt": 1_700_000_000_000i64,
            "updatedAt": 1_700_000_000_000i64
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn a_thread_nobody_has_opened_is_empty() {
        let (root, log) = scratch("empty");
        assert!(log.load(&stream("ada~bob")).is_empty());
        assert!(list_all_keys(&root).is_empty());
    }

    #[test]
    fn a_key_that_is_not_one_is_refused_rather_than_written() {
        let (root, log) = scratch("refuse");
        // Unsorted, three-sided, and a climb out of the directory: none of the
        // three spells itself the same way again, so none of them names a file.
        for key in ["bob~ada", "ada~bob~cal", "..~ada", "ada"] {
            assert!(log.append(&stream(key), &user("u1", 1, "no")).is_err());
            assert!(log.load(&stream(key)).is_empty());
        }
        assert!(list_all_keys(&root).is_empty());
    }

    #[test]
    fn later_lines_supersede_earlier_ones_by_id() {
        let (_root, log) = scratch("fold");
        for event in [
            user("u1", 1000, "can you look"),
            tool("t1", 1001, "pending"),
            tool("t1", 1003, "completed"),
        ] {
            log.append(&stream("ada~bob"), &event).unwrap();
        }

        let events = log.load(&stream("ada~bob"));
        let ids: Vec<&str> = events.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["u1", "t1"]);
        assert_eq!(events[1]["status"], "completed");
    }

    #[test]
    fn compact_rewrites_the_file_with_the_fold() {
        let (root, log) = scratch("compact");
        for event in [tool("t1", 1, "pending"), tool("t1", 2, "completed")] {
            log.append(&stream("ada~bob"), &event).unwrap();
        }
        // A thread has one file and no epochs, so the epoch it names is 1.
        assert_eq!(log.compact(&stream("ada~bob")).unwrap(), Some(1));

        assert_eq!(
            fs::read_to_string(thread_path(&root, "ada~bob").unwrap()).unwrap(),
            format!("{}\n", tool("t1", 2, "completed"))
        );
        // An empty thread has nothing to fold, and compacting it creates no file.
        assert_eq!(log.compact(&stream("cal~dee")).unwrap(), None);
        assert!(!thread_path(&root, "cal~dee").unwrap().exists());
    }

    #[test]
    fn a_label_lands_on_the_sidecar_and_a_repeat_of_it_writes_nothing() {
        let (root, _log) = scratch("label");
        let file = thread_meta_path(&root, "ada~bob").unwrap();
        fs::write(
            &file,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&meta("ada~bob")).unwrap()
            ),
        )
        .unwrap();

        set_label(&root, "ada~bob", "bob", "Bob from the other desk").unwrap();
        let written = read_meta(&root, "ada~bob").unwrap();
        assert_eq!(written["labels"]["bob"], "Bob from the other desk");

        // Compact JSON that already carries the label. A second `set_label`
        // with the same name leaves those bytes exactly as they are; had it
        // written, it would have written them out indented.
        let compact = serde_json::to_string(&written).unwrap();
        fs::write(&file, &compact).unwrap();
        set_label(&root, "ada~bob", "bob", "Bob from the other desk").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), compact);

        // No sidecar, no invented one: this function was never told who is here.
        set_label(&root, "cal~dee", "cal", "Cal").unwrap();
        assert!(!thread_meta_path(&root, "cal~dee").unwrap().exists());
    }

    #[test]
    fn a_sidecar_of_another_version_or_of_no_json_is_no_sidecar() {
        let (root, _log) = scratch("version");
        fs::write(
            thread_meta_path(&root, "ada~bob").unwrap(),
            r#"{"version":2,"a":"ada","b":"bob"}"#,
        )
        .unwrap();
        assert!(read_meta(&root, "ada~bob").is_none());

        fs::write(thread_meta_path(&root, "cal~dee").unwrap(), "{ torn").unwrap();
        assert!(read_meta(&root, "cal~dee").is_none());

        // Both still list: a name that decodes to a key is a thread, and
        // whether its sidecar reads is the reader's problem.
        let mut keys = list_all_keys(&root);
        keys.sort();
        assert_eq!(keys, ["ada~bob", "cal~dee"]);
    }

    #[test]
    fn the_startup_fold_expires_the_cards_the_restart_orphaned() {
        // The sequence at startup: for every key, expire the orphaned cards,
        // append the expiries, then compact.
        let (_root, log) = scratch("startup");
        for event in [
            user("u1", 1000, "can you look"),
            permission("p1", 1002),
            tool("t1", 1003, "completed"),
        ] {
            log.append(&stream("ada~bob"), &event).unwrap();
        }

        for expired in expire_orphaned_permissions(&log.load(&stream("ada~bob")), 2000) {
            log.append(&stream("ada~bob"), &expired).unwrap();
        }
        assert_eq!(log.compact(&stream("ada~bob")).unwrap(), Some(1));

        let events = log.load(&stream("ada~bob"));
        assert_eq!(events.len(), 3);
        assert_eq!(events[1]["decision"], "expired");
        assert_eq!(events[1]["ts"], 2000);
        // Once answered, a second startup finds nothing left to expire.
        assert!(expire_orphaned_permissions(&events, 3000).is_empty());
    }

    /// The bytes below came out of the previous Toad's own writer. Produced by
    /// running, against a throwaway `TOAD_DATA_DIR`, a script that calls
    /// `src/bun/store/threads.ts`'s `append` with these four events, then
    /// `expireOrphanedPermissions(threads.load(key), 2000)`, appends what it
    /// answers, calls `compact`, and prints the file — and then writes the
    /// sidecar with fixed timestamps and calls `setLabel(key, "bob", …)`.
    #[test]
    fn a_thread_this_writes_is_byte_for_byte_the_one_the_previous_toad_writes() {
        let (root, log) = scratch("bytes");
        for event in [
            user("u1", 1000, "can you look"),
            tool("t1", 1001, "pending"),
            permission("p1", 1002),
            tool("t1", 1003, "completed"),
        ] {
            log.append(&stream("ada~bob"), &event).unwrap();
        }
        let before = fs::read_to_string(thread_path(&root, "ada~bob").unwrap()).unwrap();
        assert_eq!(
            before,
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"can you look\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1001,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"pending\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":1002,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}]}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n"
        );

        for expired in expire_orphaned_permissions(&log.load(&stream("ada~bob")), 2000) {
            log.append(&stream("ada~bob"), &expired).unwrap();
        }
        assert_eq!(log.compact(&stream("ada~bob")).unwrap(), Some(1));
        assert_eq!(
            fs::read_to_string(thread_path(&root, "ada~bob").unwrap()).unwrap(),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"can you look\"}\n{\"kind\":\"tool\",\"id\":\"t1\",\"ts\":1003,\"toolCallId\":\"t1\",\"title\":\"run\",\"status\":\"completed\"}\n{\"kind\":\"permission\",\"id\":\"p1\",\"ts\":2000,\"requestId\":\"req-p1\",\"title\":\"read a file\",\"options\":[{\"optionId\":\"allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}],\"decision\":\"expired\"}\n"
        );

        assert_eq!(list_all_keys(&root), Vec::<String>::new());
        write_meta(&root, "ada~bob", &meta("ada~bob")).unwrap();
        set_label(&root, "ada~bob", "bob", "Bob from the other desk").unwrap();
        assert_eq!(list_all_keys(&root), ["ada~bob"]);
        assert_eq!(
            fs::read_to_string(thread_meta_path(&root, "ada~bob").unwrap()).unwrap(),
            "{\n  \"version\": 1,\n  \"a\": \"ada\",\n  \"b\": \"bob\",\n  \"sides\": {\n    \"user\": \"ada\",\n    \"agent\": \"bob\"\n  },\n  \"sessions\": [],\n  \"createdAt\": 1700000000000,\n  \"updatedAt\": 1700000000000,\n  \"labels\": {\n    \"bob\": \"Bob from the other desk\"\n  }\n}\n"
        );
    }
}
