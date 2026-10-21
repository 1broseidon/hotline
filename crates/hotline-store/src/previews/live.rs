//! A small projection of a stream, never a second cache of its folded history.
//! Log supplies an atomic seed; its existing broadcast carries later changes.

use crate::log::{Log, StreamId};
use hotline_contract::{HumanActionStatus, PasskeyAskStatus, Preview, ToolStatus, TranscriptEvent};
use serde_json::Value;
use std::collections::HashSet;
use tokio::sync::broadcast;

#[derive(Clone, Default)]
pub struct State {
    pub preview: Option<Preview>,
    pub activity: Option<(String, String)>,
    cards: HashSet<String>,
    thread_cards: HashSet<String>,
}

impl State {
    pub fn waiting(&self) -> bool {
        !self.cards.is_empty()
    }

    /// Work threads historically count permissions and human-action cards;
    /// the DM also counts passkeys and exchange pauses.
    pub fn thread_waiting(&self) -> bool {
        !self.thread_cards.is_empty()
    }

    pub fn apply(&mut self, event: &Value) -> bool {
        if let Some(preview) = super::message_preview(event) {
            self.preview = serde_json::from_value(preview).ok();
            return true;
        }
        let kind = event["kind"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "tool" | "turn" | "permission" | "human_action" | "passkey_ask" | "exchange_paused"
        ) {
            return kind == "delivery";
        }
        if let Some(id) = event["id"].as_str() {
            match kind {
                "permission" => self.set_thread_card(id, event.get("decision").is_none()),
                "human_action" => self.set_thread_card(id, event["status"] == "pending"),
                _ => {}
            }
        }
        let Ok(event) = serde_json::from_value::<TranscriptEvent>(event.clone()) else {
            return true;
        };
        match event {
            TranscriptEvent::Tool {
                tool_call_id,
                title,
                status,
                ..
            } => {
                if status == ToolStatus::InProgress {
                    self.activity = Some((tool_call_id, title));
                } else if self
                    .activity
                    .as_ref()
                    .is_some_and(|(id, _)| *id == tool_call_id)
                {
                    self.activity = None;
                }
            }
            TranscriptEvent::Turn { .. } => self.activity = None,
            TranscriptEvent::Permission { id, decision, .. } => {
                self.set_card(id, decision.is_none());
            }
            TranscriptEvent::HumanAction { id, status, .. } => {
                self.set_card(id, status == HumanActionStatus::Pending);
            }
            TranscriptEvent::PasskeyAsk { id, status, .. } => {
                self.set_card(id, status == PasskeyAskStatus::Pending);
            }
            TranscriptEvent::ExchangePaused { id, status, .. } => {
                self.set_card(id, status == hotline_contract::ExchangePauseStatus::Pending);
            }
            _ => {}
        }
        true
    }

    fn set_card(&mut self, id: String, open: bool) {
        if open {
            self.cards.insert(id);
        } else {
            self.cards.remove(&id);
        }
    }

    fn set_thread_card(&mut self, id: &str, open: bool) {
        if open {
            self.thread_cards.insert(id.to_string());
        } else {
            self.thread_cards.remove(id);
        }
    }
}

pub struct Live {
    pub state: State,
    events: broadcast::Receiver<Value>,
    stream: StreamId,
    log: Log,
}

impl Live {
    pub fn new(log: &Log, stream: StreamId) -> Self {
        let (state, events) = log.snapshot_with(&stream, |seed| {
            let mut state = State::default();
            for event in seed {
                state.apply(&event);
            }
            if let StreamId::Tape(persona_id) = &stream {
                // Activity historically follows append order, not the first
                // position of each tool id in the fold. A completed newer
                // tool must not resurrect an older overlapping tool on restart.
                // Only this cold/lag seed reads a tail; live events keep order.
                let mut raw = State::default();
                for event in super::tail(log.root(), persona_id) {
                    raw.apply(&event);
                }
                state.activity = raw.activity;
            }
            state
        });
        Self {
            state,
            events,
            stream,
            log: log.clone(),
        }
    }

    /// Drain only the burst already queued, so a busy producer cannot starve
    /// other work. Lag replaces the seed and receiver together.
    pub fn refresh(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..self.events.len() {
            match self.events.try_recv() {
                Ok(event) => changed |= self.state.apply(&event),
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    *self = Self::new(&self.log, self.stream.clone());
                    return true;
                }
                Err(_) => break,
            }
        }
        changed
    }

    pub async fn changed(&mut self) -> Option<bool> {
        let changed = match self.events.recv().await {
            Ok(event) => self.state.apply(&event),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                *self = Self::new(&self.log, self.stream.clone());
                true
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        };
        Some(self.refresh() || changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completed_overlapping_tools_do_not_reappear_on_recovery() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        let tape = StreamId::Tape("ada".into());
        let mut live = Live::new(&log, tape.clone());
        for (id, status) in [
            ("a", "in_progress"),
            ("b", "in_progress"),
            ("b", "completed"),
        ] {
            log.append(&tape, &json!({"kind":"tool", "id":id, "ts":1, "toolCallId":id, "title":id, "status":status})).unwrap();
            live.refresh();
        }
        assert!(live.state.activity.is_none());
        assert!(
            Live::new(&Log::open(root.path()), tape)
                .state
                .activity
                .is_none()
        );
    }

    #[test]
    fn thread_cards_follow_revisions_and_reseed_after_lag() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        let stream = StreamId::Side("side-1".into());
        let mut live = Live::new(&log, stream.clone());
        let mut card = json!({"kind":"permission", "id":"p", "ts":1, "requestId":"request", "title":"Allow", "options":[]});
        log.append(&stream, &card).unwrap();
        live.refresh();
        assert!(live.state.thread_waiting());
        card["decision"] = json!("allow");
        log.append(&stream, &card).unwrap();
        live.refresh();
        assert!(!live.state.thread_waiting());
        for n in 0..1_200 {
            log.append(
                &stream,
                &json!({"kind":"thought", "id":format!("th{n}"), "ts":n, "text":"thinking"}),
            )
            .unwrap();
        }
        log.append(&stream, &json!({"kind":"human_action", "id":"h", "ts":1201, "actionId":"action", "reason":"Review", "status":"pending"})).unwrap();
        live.refresh();
        assert!(live.state.thread_waiting());
        assert!(
            Live::new(&Log::open(root.path()), stream)
                .state
                .thread_waiting()
        );
    }

    #[test]
    fn a_seed_and_its_subscription_never_replay_an_older_revision() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        let stream = StreamId::Tape("ada".into());
        let first = json!({"kind":"user", "id":"first", "ts":1, "text":"first"});
        log.append(&stream, &first).unwrap();
        let (seed, mut events) = log.snapshot(&stream);
        assert_eq!(seed, [first]);
        assert!(events.try_recv().is_err());
        let next = json!({"kind":"user", "id":"next", "ts":2, "text":"next"});
        log.append(&stream, &next).unwrap();
        assert_eq!(events.try_recv().unwrap(), next);
    }
}
