//! The hourly limit on quiet turns (task-472; `docs/design/agent-inbox.md`,
//! Guards). Turns nobody watches have a ceiling: a teammate whose main
//! conversation has begun as many quiet turns in the last hour as the
//! person's `quietTurnsPerHour` (20 until they change it, Settings › General)
//! is paused before the next, with the card a pair of teammates gets ("Huck
//! paused after 20 quiet turns in an hour without you.", Keep going, Stop).
//! While it waits no quiet turn begins: its items still land, and a turn the
//! person starts hears them. Keep going counts afresh and wakes it for what
//! waited; Stop holds it until the person next writes to it, in any of its
//! threads (`Room::heard_from_person`), which counts afresh, as a line of
//! theirs does whenever the teammate is not paused.
//!
//! A quiet turn is one the inbox begins in the main conversation: a new
//! item's wake, a notice its driver did not take mid-turn, a pick-up. A work
//! thread or a job the inbox wakes is not counted: the pair's brake and the
//! job's own run bound those. The count is this process's, so a desk that
//! restarts counts afresh; the pause and the stop are kept on the room
//! stream (`quiet_limit`, by teammate) and hold across a restart.
//!
//! Each pause, this one or a pair's, adds one line to `brakes.jsonl` in the
//! data directory (`Room::note_brake`), so the number can be set from what
//! happens on a desk.

use super::{Room, lock, now_ms};
use crate::contract::{ExchangePauseStatus, ThreadId, TranscriptEvent};
use crate::log::StreamId;
use crate::session::new_id;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::Arc;

/// The span the limit counts over.
const HOUR_MS: i64 = 3_600_000;

/// What the person's answer is refused with once the pause is gone.
const GONE: &str = "That pause is no longer waiting for an answer.";

/// The quiet turns a teammate's main conversation began within the last
/// hour, oldest first.
#[derive(Default)]
pub(in crate::session) struct QuietTurns(VecDeque<i64>);

impl QuietTurns {
    /// Whether one more may begin at `now` under `limit` an hour, counting
    /// it when it may.
    fn begin(&mut self, now: i64, limit: usize) -> bool {
        while self.0.front().is_some_and(|at| *at <= now - HOUR_MS) {
            self.0.pop_front();
        }
        if self.0.len() >= limit {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

/// How a teammate is held at its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::session) enum Held {
    /// Paused, until the person answers the card.
    Paused,
    /// Stopped by the person, until they next write to it.
    Stopped,
}

/// The id of a teammate's record on the room stream, which folds by id
/// alone, so it is never a teammate's own.
fn record_id(persona_id: &str) -> String {
    format!("quiet-limit:{persona_id}")
}

impl Room {
    /// The person's limit: how many quiet turns a teammate may begin in an
    /// hour.
    fn quiet_limit(&self) -> usize {
        let limit = crate::room::settings(&self.log)
            .get(crate::room::QUIET_TURNS_PER_HOUR)
            .and_then(Value::as_i64)
            .filter(|limit| (1..=crate::room::MAX_QUIET_TURNS).contains(limit))
            .unwrap_or(crate::room::DEFAULT_QUIET_TURNS);
        usize::try_from(limit).unwrap_or(usize::MAX)
    }

    /// The teammate's record at its limit, as the room stream keeps it.
    fn quiet_record(&self, persona_id: &str) -> Option<Value> {
        let id = record_id(persona_id);
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .rfind(|event| event["kind"] == "quiet_limit" && event["id"] == id.as_str())
    }

    /// Whether the teammate is held at its limit. A hold a newer Hotline
    /// wrote that this one cannot read holds as a pause does, until the
    /// person answers.
    pub(in crate::session) fn quiet_held(&self, persona_id: &str) -> Option<Held> {
        match self.quiet_record(persona_id)?["held"].as_str()? {
            "stopped" => Some(Held::Stopped),
            _ => Some(Held::Paused),
        }
    }

    /// Writes how the teammate is held, keeping whatever else its record
    /// holds.
    fn hold_quiet(&self, persona_id: &str, held: Option<Held>) {
        let mut record = self.quiet_record(persona_id).unwrap_or_else(
            || json!({"kind": "quiet_limit", "id": record_id(persona_id), "teammate": persona_id}),
        );
        record["held"] = match held {
            Some(Held::Paused) => json!("paused"),
            Some(Held::Stopped) => json!("stopped"),
            None => Value::Null,
        };
        if let Err(error) = self.log.append(&StreamId::Room, &record) {
            eprintln!("a teammate's quiet-turn limit could not be written: {error}");
        }
    }

    /// A quiet turn of `persona_id`'s main conversation is about to begin:
    /// whether it may. While the teammate is held none may; one past the
    /// limit pauses it instead, with the card, and is noted.
    pub(in crate::session) fn quiet_turn_may_begin(&self, persona_id: &str) -> bool {
        if self.quiet_held(persona_id).is_some() {
            return false;
        }
        let limit = self.quiet_limit();
        let turns = {
            let mut counts = lock(&self.quiet_turns);
            let turns = counts.entry(persona_id.to_string()).or_default();
            if turns.begin(now_ms(), limit) {
                return true;
            }
            turns.0.len()
        };
        self.hold_quiet(persona_id, Some(Held::Paused));
        self.write(
            persona_id,
            &TranscriptEvent::QuietPaused {
                id: format!("quiet-paused:{}", new_id()),
                ts: now_ms(),
                turns: u32::try_from(turns).unwrap_or(u32::MAX),
                status: ExchangePauseStatus::Pending,
            },
        );
        self.note_brake(json!({
            "brake": "quiet",
            "teammate": persona_id,
            "count": turns,
            "limit": limit,
        }));
        false
    }

    /// The person's answer to the card: Keep going counts afresh and wakes
    /// the teammate for what waited; Stop holds it until they next write to
    /// it.
    pub fn answer_quiet_pause(
        self: &Arc<Self>,
        persona_id: &str,
        keep_going: bool,
    ) -> Result<(), String> {
        self.persona(persona_id)?;
        if self.quiet_held(persona_id) != Some(Held::Paused) {
            return Err(GONE.to_string());
        }
        lock(&self.quiet_turns).remove(persona_id);
        let (held, status) = if keep_going {
            (None, ExchangePauseStatus::Resumed)
        } else {
            (Some(Held::Stopped), ExchangePauseStatus::Stopped)
        };
        self.hold_quiet(persona_id, held);
        for mut card in self
            .tape(persona_id)
            .into_iter()
            .filter(|event| event["kind"] == "quiet_paused" && event["status"] == "pending")
        {
            card["status"] = json!(status);
            self.write_value(persona_id, &card);
        }
        if keep_going {
            self.inbox_rewake(persona_id, &ThreadId::dm(persona_id));
        }
        Ok(())
    }

    /// The person wrote to `persona_id`: its quiet turns count afresh, and a
    /// Stop at its limit is lifted. A pause still waits for its card.
    pub(in crate::session) fn quiet_heard_from_person(&self, persona_id: &str) {
        match self.quiet_held(persona_id) {
            Some(Held::Paused) => {}
            Some(Held::Stopped) => {
                lock(&self.quiet_turns).remove(persona_id);
                self.hold_quiet(persona_id, None);
            }
            None => {
                lock(&self.quiet_turns).remove(persona_id);
            }
        }
    }

    /// One line in `brakes.jsonl` in the data directory: a brake paused
    /// teammates, when, and at what count.
    pub(in crate::session) fn note_brake(&self, mut line: Value) {
        line["at"] = json!(now_ms());
        let path = self.log.root().join("brakes.jsonl");
        let noted = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| writeln!(file, "{line}"));
        if let Err(error) = noted {
            eprintln!("a brake could not be noted in {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The limit counts the quiet turns of the last hour: one past it is
    /// refused, and an hour after the first, one more may begin.
    #[test]
    fn the_limit_counts_the_last_hours_quiet_turns() {
        let mut turns = QuietTurns::default();
        let start = 1_000_000;
        assert!(turns.begin(start, 3));
        assert!(turns.begin(start + 10_000, 3));
        assert!(turns.begin(start + 20_000, 3));
        assert!(!turns.begin(start + 30_000, 3), "a fourth in the hour");
        assert!(!turns.begin(start + HOUR_MS - 1, 3));
        assert!(turns.begin(start + HOUR_MS, 3), "the first is an hour old");
        assert!(!turns.begin(start + HOUR_MS + 1, 3));
        assert!(turns.begin(start + HOUR_MS + 10_000, 3));
    }
}
