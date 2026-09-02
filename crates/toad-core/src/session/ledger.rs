//! What tools a teammate got, from where, and — for anything absent — why.
//!
//! The bug this exists to make impossible: a tool disappears and nothing
//! says so. A row always carries a reason, in every state, because an
//! optional explanation is the one nobody fills in. A ledger is built per
//! session, at start, from the same arrays the session hands the agent, and
//! published here so the UI and the verify harnesses can read it without
//! holding the session. It outlives the session on purpose: the question
//! "why does this teammate not have that tool" is usually asked after the
//! teammate has been stopped again.

use crate::contract::{AgentKind, TeammateToolLedger, ToolLedgerRow, ToolSourceKind, ToolState};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// Said instead of nothing, when a caller manages to supply no reason at all.
const NO_REASON: &str = "no reason was recorded, which is itself a bug in Toad";

fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn row_key(source: ToolSourceKind, origin: &str, name: &str) -> String {
    format!("{source:?}{origin}{name}")
}

static LEDGERS: Mutex<Option<HashMap<String, TeammateToolLedger>>> = Mutex::new(None);

fn store() -> std::sync::MutexGuard<'static, Option<HashMap<String, TeammateToolLedger>>> {
    lock(&LEDGERS)
}

/// What Toad knows about this teammate's tools. `None` when it has never
/// started under a Toad that keeps a ledger — which is itself worth saying
/// out loud in the UI rather than drawing an empty table.
pub fn teammate_tools(persona_id: &str) -> Option<TeammateToolLedger> {
    store()
        .as_ref()
        .and_then(|ledgers| ledgers.get(persona_id).cloned())
}

pub fn forget(persona_id: &str) {
    if let Some(ledgers) = store().as_mut() {
        ledgers.remove(persona_id);
    }
}

/// What tools a teammate got, built at session start.
pub struct ToolLedger {
    persona_id: String,
    agent_kind: AgentKind,
    backend_id: String,
    at: i64,
    rows: HashMap<String, ToolLedgerRow>,
}

impl ToolLedger {
    pub fn new(
        persona_id: impl Into<String>,
        agent_kind: AgentKind,
        backend_id: impl Into<String>,
    ) -> Self {
        Self {
            persona_id: persona_id.into(),
            agent_kind,
            backend_id: backend_id.into(),
            at: now_ms(),
            rows: HashMap::new(),
        }
    }

    fn put(
        &mut self,
        state: ToolState,
        source: ToolSourceKind,
        origin: impl Into<String>,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) -> &mut Self {
        let origin = origin.into();
        let name = name.into();
        let reason = reason.into();
        let trimmed = reason.trim();
        self.rows.insert(
            row_key(source, &origin, &name),
            ToolLedgerRow {
                name,
                source,
                origin,
                state,
                reason: if trimmed.is_empty() {
                    NO_REASON.to_string()
                } else {
                    trimmed.to_string()
                },
                at: now_ms(),
            },
        );
        self
    }

    /// Toad watched the agent take this tool.
    pub fn verified(
        &mut self,
        source: ToolSourceKind,
        origin: impl Into<String>,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) -> &mut Self {
        self.put(ToolState::Verified, source, origin, name, reason)
    }

    /// Toad handed this tool over and cannot see whether the agent took it.
    pub fn declared(
        &mut self,
        source: ToolSourceKind,
        origin: impl Into<String>,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) -> &mut Self {
        self.put(ToolState::Declared, source, origin, name, reason)
    }

    /// This tool is not there. `reason` is the point of the call.
    pub fn absent(
        &mut self,
        source: ToolSourceKind,
        origin: impl Into<String>,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) -> &mut Self {
        self.put(ToolState::Absent, source, origin, name, reason)
    }

    /// The same state for a list of names from one supplier.
    pub fn all(
        &mut self,
        state: ToolState,
        source: ToolSourceKind,
        origin: &str,
        names: &[&str],
        reason: &str,
    ) -> &mut Self {
        for name in names {
            self.put(state, source, origin, *name, reason);
        }
        self
    }

    pub fn snapshot(&self) -> TeammateToolLedger {
        TeammateToolLedger {
            persona_id: self.persona_id.clone(),
            agent_kind: self.agent_kind,
            backend_id: self.backend_id.clone(),
            at: self.at,
            rows: self.rows.values().cloned().collect(),
        }
    }

    /// Makes this ledger the answer `teammate.tools` gives for its teammate.
    pub fn publish(&self) -> TeammateToolLedger {
        let snapshot = self.snapshot();
        store()
            .get_or_insert_with(HashMap::new)
            .insert(self.persona_id.clone(), snapshot.clone());
        snapshot
    }
}

/// Promote rows from `declared` to `verified`, because the agent was seen
/// asking for them.
pub fn mark_verified(
    persona_id: &str,
    source: ToolSourceKind,
    origin: &str,
    names: &[&str],
    reason: &str,
) {
    let mut held = store();
    let Some(ledgers) = held.as_mut() else {
        return;
    };
    let Some(ledger) = ledgers.get_mut(persona_id) else {
        return;
    };
    let wanted: std::collections::HashSet<&str> = names.iter().copied().collect();
    let at = now_ms();
    for row in &mut ledger.rows {
        if row.source != source || row.origin != origin || !wanted.contains(row.name.as_str()) {
            continue;
        }
        row.state = ToolState::Verified;
        row.reason = reason.to_string();
        row.at = at;
    }
}

/// Flip a supplier's rows to absent with one cause — a server that went
/// away mid-session.
pub fn mark_absent(persona_id: &str, source: ToolSourceKind, origin: &str, reason: &str) {
    let mut held = store();
    let Some(ledgers) = held.as_mut() else {
        return;
    };
    let Some(ledger) = ledgers.get_mut(persona_id) else {
        return;
    };
    let at = now_ms();
    for row in &mut ledger.rows {
        if row.source == source && row.origin == origin {
            row.state = ToolState::Absent;
            row.reason = reason.to_string();
            row.at = at;
        }
    }
}

/// Every teammate whose ledger mentions this supplier — the teardown handle.
#[cfg(test)]
pub fn mentioning(source: ToolSourceKind, origin: &str) -> Vec<String> {
    store()
        .as_ref()
        .into_iter()
        .flatten()
        .filter(|(_, ledger)| {
            ledger
                .rows
                .iter()
                .any(|row| row.source == source && row.origin == origin)
        })
        .map(|(persona_id, _)| persona_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_none_before_the_teammate_has_ever_started() {
        assert!(teammate_tools("nobody").is_none());
    }

    #[test]
    fn every_row_carries_a_reason_in_every_state() {
        let mut ledger = ToolLedger::new("p1", AgentKind::Pi, "pi");
        ledger
            .verified(
                ToolSourceKind::Builtin,
                "pi",
                "read",
                "a built-in of the Toad Agent runtime",
            )
            .declared(
                ToolSourceKind::Mcp,
                "Echo",
                "echo__shout",
                "handed to the backend as a descriptor",
            )
            .absent(
                ToolSourceKind::Mcp,
                "Toad",
                "hop_desk",
                "this Toad does not own the bridge socket",
            )
            .publish();
        let ledger = teammate_tools("p1").expect("published");
        assert_eq!(ledger.rows.len(), 3);
        for row in &ledger.rows {
            assert!(!row.reason.is_empty());
        }
    }

    #[test]
    fn an_empty_reason_becomes_a_loud_one_rather_than_an_empty_cell() {
        let mut ledger = ToolLedger::new("p2", AgentKind::Pi, "pi");
        ledger
            .absent(ToolSourceKind::Mcp, "Echo", "shout", "   ")
            .publish();
        let reason = &teammate_tools("p2").unwrap().rows[0].reason;
        assert!(reason.contains("bug in Toad"), "{reason}");
    }

    #[test]
    fn the_same_tool_from_two_suppliers_is_two_rows() {
        let mut ledger = ToolLedger::new("p3", AgentKind::Pi, "pi");
        ledger
            .verified(ToolSourceKind::Mcp, "A", "search", "from A")
            .verified(ToolSourceKind::Mcp, "B", "search", "from B")
            .publish();
        assert_eq!(teammate_tools("p3").unwrap().rows.len(), 2);
    }

    #[test]
    fn a_declared_row_becomes_verified_when_the_agent_is_seen_asking() {
        let mut ledger = ToolLedger::new("p4", AgentKind::Acp, "cursor");
        ledger
            .declared(
                ToolSourceKind::Mcp,
                "Echo",
                "echo__shout",
                "handed over as a descriptor",
            )
            .publish();
        mark_verified(
            "p4",
            ToolSourceKind::Mcp,
            "Echo",
            &["echo__shout"],
            "the agent listed tools on this teammate's own endpoint",
        );
        let row = &teammate_tools("p4").unwrap().rows[0];
        assert_eq!(row.state, ToolState::Verified);
        assert!(row.reason.contains("own endpoint"));
    }

    #[test]
    fn a_supplier_that_goes_away_turns_its_rows_absent_with_one_cause() {
        let mut ledger = ToolLedger::new("p5", AgentKind::Pi, "pi");
        ledger
            .verified(ToolSourceKind::Mcp, "Echo", "echo__shout", "attached")
            .verified(ToolSourceKind::Builtin, "pi", "read", "a built-in")
            .publish();
        mark_absent(
            "p5",
            ToolSourceKind::Mcp,
            "Echo",
            "the Echo server is configured but not answering",
        );
        let rows = teammate_tools("p5").unwrap().rows;
        let shout = rows.iter().find(|row| row.name == "echo__shout").unwrap();
        assert_eq!(shout.state, ToolState::Absent);
        let read = rows.iter().find(|row| row.name == "read").unwrap();
        assert_eq!(read.state, ToolState::Verified);
    }

    #[test]
    fn an_observation_about_a_teammate_with_no_ledger_is_a_no_op() {
        mark_verified("gone", ToolSourceKind::Mcp, "x", &["y"], "z");
    }

    #[test]
    fn mentioning_names_every_teammate_whose_ledger_holds_the_supplier() {
        let mut a = ToolLedger::new("mention-a", AgentKind::Pi, "pi");
        a.verified(ToolSourceKind::Mcp, "EchoMention", "t", "attached")
            .publish();
        let mut b = ToolLedger::new("mention-b", AgentKind::Acp, "cursor");
        b.declared(ToolSourceKind::Mcp, "EchoMention", "t", "handed over")
            .publish();
        let mut c = ToolLedger::new("mention-c", AgentKind::Pi, "pi");
        c.verified(ToolSourceKind::Builtin, "pi", "read", "built-in")
            .publish();
        let mut found = mentioning(ToolSourceKind::Mcp, "EchoMention");
        found.sort();
        assert_eq!(found, ["mention-a", "mention-b"]);
    }
}
