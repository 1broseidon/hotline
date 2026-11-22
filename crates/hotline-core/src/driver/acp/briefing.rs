//! Whether a session already holds its briefing, and when it stops holding
//! it.
//!
//! The briefing rides as an ordinary user message, so two things decide
//! whether the agent still has it. A session reopened on a new process keeps
//! everything it was told, the briefing included; and a compaction summarises
//! the briefing away with the rest of the conversation. Claude's adapter
//! drops it outright (asked on 2026-11-21 for the closing line of the
//! briefing after `/compact`, claude-agent-acp 0.78.0 answered that it was
//! not there). Codex's keeps recent user messages verbatim up to a budget,
//! so a short session's briefing survives, and a long one's, the oldest of
//! them, goes first.
//!
//! What is remembered is a hash of the standing briefing: the text without
//! the parts that describe the moment it was built (the wake block's elapsed
//! time, the seeded transcript). One small file per backend and session,
//! under the data directory's cache. A missing or unreadable file is a
//! briefing not sent; a file that cannot be written costs one extra briefing,
//! never a missing one.

use crate::paths;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The backends whose compaction Hotline sees, each confirmed on the wire on
/// 2026-11-21 (`tests/fixtures/acp/`): a reopened session of one of these
/// is not briefed again when it holds the same briefing, because a
/// compaction that drops it will be seen and answered. Any other backend is
/// briefed on every new process as before, since that is the only thing that
/// ever gives a compacted agent its briefing back.
const SIGNALS_COMPACTION: &[&str] = &["claude-acp", "codex-acp"];

/// Where whether `session_id` on `backend_id` holds its briefing is kept, for
/// a backend whose compaction Hotline sees; `None` for any other.
pub(super) fn record(root: &Path, backend_id: &str, session_id: &str) -> Option<PathBuf> {
    SIGNALS_COMPACTION.contains(&backend_id).then(|| {
        // Provider session ids are the provider's own strings: hashed, so
        // none of them can name a path.
        let name = hex::encode(Sha256::digest(format!("{backend_id}\n{session_id}")));
        paths::acp_briefed_dir(root).join(name)
    })
}

/// The hash a briefing is remembered by.
pub(crate) fn digest(standing: &str) -> String {
    hex::encode(Sha256::digest(standing.as_bytes()))
}

/// Whether the session the record is for was last briefed with `standing`.
pub(super) fn holds(record: Option<&Path>, standing: &str) -> bool {
    record
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|held| held.trim() == digest(standing))
}

/// Writes down that the session was briefed with the standing text hashed
/// to `digest`.
pub(super) fn remember(record: &Path, digest: &str) {
    let written = record
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(record, digest));
    if let Err(error) = written {
        eprintln!(
            "[acp] {} could not be written, so the next start briefs again: {error}",
            record.display()
        );
    }
}

/// Forgets the session's briefing: its agent no longer has it.
pub(super) fn forget(record: &Path) {
    match std::fs::remove_file(record) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("[acp] {} could not be removed: {error}", record.display()),
    }
}

/// Whether a raw `session/update` says the agent finished compacting its
/// context. Both adapters say it as a tool call tagged
/// `_meta.contextCompaction`, ended `completed` or `failed`; Claude's then
/// sends one more update, with no status, carrying the token counts. Only a
/// completed one dropped anything.
pub(super) fn completed_compaction(update: &Value) -> bool {
    matches!(
        update.get("sessionUpdate").and_then(Value::as_str),
        Some("tool_call" | "tool_call_update")
    ) && update
        .pointer("/_meta/contextCompaction")
        .is_some_and(Value::is_object)
        && update.get("status").and_then(Value::as_str) == Some("completed")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The updates of one captured compaction, as the adapter sent them.
    fn captured(jsonl: &str) -> Vec<Value> {
        jsonl
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["params"]["update"].clone())
            .collect()
    }

    /// The real shapes, captured on this desk from `/compact`: Claude's
    /// opening call, its completion, and the trailing token report; Codex's
    /// opening call and completion. Exactly one update in each says the
    /// compaction completed.
    #[test]
    fn a_captured_compaction_completes_once() {
        for (adapter, jsonl, at) in [
            (
                "claude-agent-acp 0.78.0",
                include_str!("../../../tests/fixtures/acp/claude-acp-0.78.0-compact.jsonl"),
                1,
            ),
            (
                "codex-acp 1.12.0",
                include_str!("../../../tests/fixtures/acp/codex-acp-1.12.0-compact.jsonl"),
                1,
            ),
        ] {
            let completed: Vec<usize> = captured(jsonl)
                .iter()
                .enumerate()
                .filter(|(_, update)| completed_compaction(update))
                .map(|(index, _)| index)
                .collect();
            assert_eq!(completed, [at], "{adapter}");
        }
    }

    /// A failed compaction dropped nothing, and an ordinary tool call that
    /// completes is not a compaction.
    #[test]
    fn only_a_completed_compaction_counts() {
        let failed = serde_json::json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c",
            "status": "failed",
            "_meta": {"contextCompaction": {"version": 1}},
        });
        let ordinary = serde_json::json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c",
            "status": "completed",
            "_meta": {"claudeCode": {"toolName": "Bash"}},
        });
        assert!(!completed_compaction(&failed));
        assert!(!completed_compaction(&ordinary));
    }

    #[test]
    fn only_a_backend_whose_compaction_is_seen_keeps_a_record() {
        let root = Path::new("/data");
        assert!(record(root, "claude-acp", "s").is_some());
        assert!(record(root, "codex-acp", "s").is_some());
        assert!(record(root, "cursor", "s").is_none());
        assert_ne!(
            record(root, "claude-acp", "s"),
            record(root, "codex-acp", "s")
        );
        let path = record(root, "claude-acp", "../../etc/passwd").unwrap();
        assert_eq!(path.parent(), Some(paths::acp_briefed_dir(root).as_path()));
    }
}
