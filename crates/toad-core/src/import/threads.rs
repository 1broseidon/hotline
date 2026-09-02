//! Peer threads, copied from the previous Toad's `threads/` directory.
//!
//! A tape that came over can name a `threadKey`. The window draws a card
//! for that key and opens the stream; if the stream is not here, the card
//! is a conversation that lied. The sidecar format is already the previous
//! Toad's, so the copy is the files themselves, not a rewrite.

use super::{Report, Skipped};
use crate::log::{Log, thread};
use crate::paths::{
    decode_file_component, thread_meta_path, thread_participants, thread_path, threads_dir,
};
use crate::room;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

/// Copies every thread whose key names at least one teammate in this room.
///
/// Already-present teammates count: a second import, or a thread with
/// someone who was already on the roster, still has a card to open.
/// A thread that already exists here is left alone. A key whose sides
/// are both strangers is skipped.
pub(super) fn import(from: &Path, log: &Log, report: &mut Report) -> io::Result<()> {
    let known: HashSet<String> = room::roster(log)
        .into_iter()
        .map(|persona| persona.id)
        .collect();
    let dest = log.root();
    for key in source_keys(from) {
        let Some((a, b)) = thread_participants(&key) else {
            continue;
        };
        if !known.contains(a) && !known.contains(b) {
            report.skipped.push(Skipped {
                item: format!("thread {key}"),
                reason: "names no teammate in this room".into(),
            });
            continue;
        }
        if thread_exists(dest, &key) {
            report.skipped.push(Skipped {
                item: format!("thread {key}"),
                reason: "already exists here".into(),
            });
            continue;
        }
        if copy_thread(from, dest, &key)? {
            report.threads += 1;
        }
    }
    Ok(())
}

/// Sidecars first — a conversation exists from the moment it is opened —
/// then a `.jsonl` whose sidecar was never written, so a tape that named
/// that key still has a stream to open.
fn source_keys(from: &Path) -> Vec<String> {
    let mut keys = thread::list_all_keys(from);
    let Ok(entries) = fs::read_dir(threads_dir(from)) else {
        return keys;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".jsonl")) else {
            continue;
        };
        let key = decode_file_component(stem);
        if thread_path(from, &key).is_some() && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn thread_exists(root: &Path, key: &str) -> bool {
    thread_meta_path(root, key).is_some_and(|path| path.is_file())
        || thread_path(root, key).is_some_and(|path| path.is_file())
}

/// Copies the sidecar and the stream, whichever of the two the source has.
/// Bytes are unchanged. Returns whether anything was copied.
fn copy_thread(from: &Path, to: &Path, key: &str) -> io::Result<bool> {
    let mut copied = false;
    copied |= copy_if_present(thread_meta_path(from, key), thread_meta_path(to, key))?;
    copied |= copy_if_present(thread_path(from, key), thread_path(to, key))?;
    Ok(copied)
}

fn copy_if_present(
    from: Option<std::path::PathBuf>,
    to: Option<std::path::PathBuf>,
) -> io::Result<bool> {
    let (Some(from), Some(to)) = (from, to) else {
        return Ok(false);
    };
    if !from.is_file() {
        return Ok(false);
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{McpPolicy, Persona, PolicyMode};
    use crate::log::Log;
    use serde_json::json;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toad-core-import-threads-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn teammate(id: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: id.to_string(),
            goal: String::new(),
            face: None,
            team: None,
            backend_id: "pi".to_string(),
            cwd: format!("/tmp/{id}"),
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
            created_at: 1,
            updated_at: 1,
        }
    }

    fn write_thread(root: &Path, key: &str, line: &str) {
        let (a, b) = key.split_once('~').unwrap();
        fs::create_dir_all(threads_dir(root)).unwrap();
        fs::write(thread_path(root, key).unwrap(), line).unwrap();
        fs::write(
            thread_meta_path(root, key).unwrap(),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "version": 1,
                    "a": a,
                    "b": b,
                    "sides": { "user": a, "agent": b },
                    "sessions": [],
                    "createdAt": 1,
                    "updatedAt": 1,
                }))
                .unwrap()
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_thread_naming_a_teammate_here_is_copied_and_a_stranger_is_skipped() {
        let from = scratch("from");
        let line = "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1,\"text\":\"hello\"}\n";
        write_thread(&from, "ada~bob", line);
        write_thread(&from, "ada~ghost", line);
        write_thread(&from, "nobody~stranger", line);

        let dest = scratch("dest");
        let log = Log::open(&dest);
        crate::room::append_persona(&log, &teammate("ada")).unwrap();
        crate::room::append_persona(&log, &teammate("bob")).unwrap();

        let mut report = Report::default();
        import(&from, &log, &mut report).unwrap();

        assert_eq!(report.threads, 2, "{report:?}");
        assert_eq!(
            fs::read(thread_path(&dest, "ada~bob").unwrap()).unwrap(),
            fs::read(thread_path(&from, "ada~bob").unwrap()).unwrap()
        );
        assert_eq!(
            fs::read(thread_meta_path(&dest, "ada~bob").unwrap()).unwrap(),
            fs::read(thread_meta_path(&from, "ada~bob").unwrap()).unwrap()
        );
        assert_eq!(
            fs::read(thread_path(&dest, "ada~ghost").unwrap()).unwrap(),
            fs::read(thread_path(&from, "ada~ghost").unwrap()).unwrap()
        );
        assert!(!thread_path(&dest, "nobody~stranger").unwrap().exists());
        assert!(
            report
                .skipped
                .iter()
                .any(|skipped| skipped.item == "thread nobody~stranger"
                    && skipped.reason == "names no teammate in this room"),
            "{report:?}"
        );

        let second = {
            let mut report = Report::default();
            import(&from, &log, &mut report).unwrap();
            report
        };
        assert_eq!(second.threads, 0, "{second:?}");
        assert!(
            second
                .skipped
                .iter()
                .any(|skipped| skipped.item == "thread ada~bob"
                    && skipped.reason == "already exists here"),
            "{second:?}"
        );
    }
}
