//! What the roster row and the thread lists are told of a teammate's live
//! threads: which run a turn, and what each is called.

use super::*;

impl Room {
    /// Whether this work thread has a turn in flight, without listing its history.
    pub(crate) fn work_thread_working(&self, thread: &ThreadId) -> bool {
        self.sides
            .get(&thread.key)
            .is_some_and(|live| live.thread == *thread && live.working())
    }

    /// Call threads with a turn in flight, separate from their saved lifetime.
    pub fn working_calls(&self, persona_id: &str) -> Vec<String> {
        self.sides
            .of(persona_id)
            .iter()
            .filter(|side| side.thread.kind == ThreadKind::Call && side.working())
            .map(|side| side.id.clone())
            .collect()
    }

    pub fn sides(&self, persona_id: &str) -> Vec<RunningSide> {
        self.sides
            .of(persona_id)
            .iter()
            .filter(|side| side.thread.kind == ThreadKind::Side)
            .map(|side| RunningSide {
                side_id: side.id.clone(),
                title: lock(&side.title).clone(),
                started_at: side.started,
                working: side.turn_running(),
            })
            .collect()
    }

    /// Every side thread this teammate has had: the live ones, oldest first,
    /// then the parked, then the archived, each newest first.
    pub fn side_threads(&self, persona_id: &str) -> Vec<SideThreadSummary> {
        let mut live: Vec<SideThreadSummary> = self
            .sides
            .of(persona_id)
            .iter()
            .filter(|side| side.thread.kind == ThreadKind::Side)
            .map(|side| self.side_summary(side))
            .collect();
        let mut kept: Vec<SideThreadSummary> = self
            .stored_side_ids()
            .into_iter()
            .filter(|id| self.sides.get(id).is_none())
            .filter_map(|id| self.stored_summary(&id, persona_id))
            .collect();
        kept.sort_by_key(|summary| {
            (
                summary.status == SideStatus::Archived,
                std::cmp::Reverse(summary.archived_at.unwrap_or(summary.last_at)),
            )
        });
        live.extend(kept);
        live
    }

    /// Whether a live side thread of this teammate is stopped on a card, for
    /// the roster row.
    pub(in crate::session) fn side_cards_waiting(&self, persona_id: &str) -> bool {
        self.sides.of(persona_id).iter().any(|side| {
            crate::session::threads::cards_waiting(&self.log, &side.thread, &side.roster)
        })
    }
}
