//! The pairs' threads as a window reads them, and what a restart left
//! undelivered.

use super::*;
use crate::paths::thread_participants;

impl Room {
    /// Every thread this teammate is in, newest first.
    pub fn peer_threads(&self, persona_id: &str) -> Vec<PeerThreadSummary> {
        let names: HashMap<String, String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| (persona.id, persona.name))
            .collect();
        let named = |id: &str| {
            names
                .get(id)
                .cloned()
                .or_else(|| {
                    linked::is_linked(id)
                        .then(|| {
                            self.participant(id)
                                .ok()
                                .map(|p| crate::session::links::shown(&p))
                        })
                        .flatten()
                })
                .unwrap_or_else(|| "Deleted teammate".to_string())
        };
        let mut summaries: Vec<PeerThreadSummary> = thread::keys_for(self.log.root(), persona_id)
            .into_iter()
            .filter_map(|key| {
                let (a, b) = thread_participants(&key)?;
                let other = if a == persona_id { b } else { a };
                let (user_side, agent_side) = (a.to_string(), b.to_string());
                let events = self.log.load(&StreamId::Pair(key.clone()));
                let last = events
                    .iter()
                    .rfind(|event| matches!(kind_of(event), "user" | "agent"));
                Some(PeerThreadSummary {
                    with_persona_id: other.to_string(),
                    with_name: named(other),
                    exchanges: events
                        .iter()
                        .filter(|event| kind_of(event) == "turn")
                        .count() as i64,
                    last_at: events
                        .iter()
                        .filter_map(|event| event.get("ts").and_then(Value::as_i64))
                        .max()
                        .unwrap_or_default(),
                    waiting: events.iter().any(|event| {
                        kind_of(event) == "permission" && event.get("decision").is_none()
                    }),
                    working_persona_id: None,
                    preview: last.map(|event| PeerPreview {
                        from_name: named(if kind_of(event) == "user" {
                            &user_side
                        } else {
                            &agent_side
                        }),
                        text: event
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        at: event.get("ts").and_then(Value::as_i64).unwrap_or_default(),
                    }),
                    thread_key: key,
                })
            })
            .collect();
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.last_at));
        summaries
    }

    /// Says that these messages have been read, and answers how many of them
    /// that moved. An id naming nothing, or a message that is already read,
    /// moves nothing — which is what makes a repeated receipt harmless.
    pub fn mark_peer_read(&self, key: &str, event_ids: &[String]) -> usize {
        let stream = StreamId::Pair(key.to_string());
        let events = self.log.load(&stream);
        let updates = read_receipt_updates(&events, event_ids);
        let moved = updates.len();
        let answerer = thread_participants(key).map_or("", |(first, _)| first);
        for event in updates {
            self.write_thread(key, answerer, &event);
        }
        moved
    }

    /// What a restart left undone, put right once when the room opens.
    ///
    /// A delivery on a tape that the agent never read is handed to it again,
    /// so an answer that landed as the desk went down is still heard, once.
    pub(in crate::session) async fn recover_exchanges(self: &Arc<Self>) {
        self.recover_queued_exchanges();
        for persona in room::roster(&self.log) {
            let tape = self.tape(&persona.id);
            for (id, _ts, cause, text) in unheard(&tape) {
                // A handoff and a question are the pair worker's to deliver,
                // with the request their turn answers.
                if matches!(
                    cause,
                    DeliveryCause::Handoff { .. } | DeliveryCause::Question { .. }
                ) {
                    continue;
                }
                // Use the same idempotent dispatch as the durable result worker:
                // either may see the saved result first at startup.
                if let Err(error) = self
                    .deliver_identified(&persona.id, &id, cause, text, None)
                    .await
                {
                    eprintln!(
                        "{}'s delivery could not be handed on: {error}",
                        persona.name
                    );
                }
            }
        }
    }

    /// One line onto the thread's stream, through the shared write path: it
    /// is indexed under both teammates, and a card in it is pushed to the
    /// person and counted for `answerer`, who is the one it is waiting on.
    pub(super) fn write_thread(&self, key: &str, answerer: &str, event: &TranscriptEvent) {
        self.threads().write(&ThreadId::pair(key), answerer, event);
    }
}
