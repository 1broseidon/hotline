//! A small projection of a stream, never a second cache of its folded history.
//! Log supplies an atomic seed; its existing broadcast carries later changes.

use crate::log::{Log, StreamId};
use hotline_contract::{HumanActionStatus, PasskeyAskStatus, Preview, TranscriptEvent};
use serde_json::Value;
use std::collections::HashSet;
use tokio::sync::broadcast;

#[derive(Clone, Default)]
pub struct State {
    pub preview: Option<Preview>,
    pub turn: Turn,
    cards: HashSet<String>,
    thread_cards: HashSet<String>,
}

/// What the tape says of the turn in flight: the facts the words beside a
/// teammate's mark are read from (`activity_line` in the core). Folded in
/// append order, so a tool that finished is never seen running again, and
/// cleared by the line that ends a turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    /// The newest words the teammate gave `activity` since the turn began.
    /// A turn held open for its helpers keeps them through the lines the
    /// person sends meanwhile: they say what it is still waiting on.
    pub declared: Option<String>,
    /// The part of the turn answering the lines the agent has now.
    pub ask: Ask,
    /// The part that will answer the lines written while the agent was still
    /// on the ones before: what happened since they were written, taken up
    /// as the ask once the agent reads one. A line read is marked on the tape
    /// when the agent's next update is heard, which can be after the tool
    /// that update reports has already written its own line (`activity`
    /// writes its words at once), so what came after the line was written is
    /// kept for it rather than started afresh.
    next: Option<Ask>,
}

/// The part of a turn answering the lines the agent has read: the person's,
/// or a colleague's delivery.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ask {
    /// The ids of the lines it answers.
    lines: HashSet<String>,
    /// The newest words the teammate gave `activity` in this ask; a call
    /// with a kind and no words clears the ones before it.
    pub declared: Option<String>,
    /// The tool calls still in flight, other than the calls that declare.
    steps: HashSet<String>,
    /// The agent has given anything back: it read a line, or wrote a
    /// thought, a reply, a step or a declaration.
    pub heard: bool,
}

impl Ask {
    /// A step is in flight.
    pub fn stepping(&self) -> bool {
        !self.steps.is_empty()
    }

    /// Folds in one line of what the agent did.
    fn apply(&mut self, kind: &str, event: &Value) {
        match kind {
            "activity" => {
                self.declared = words(event);
                self.heard = true;
            }
            "tool" => {
                self.heard = true;
                let declares =
                    matches!(event["said"]["key"].as_str(), Some("activity" | "outcome"));
                if let Some(call) = event["toolCallId"].as_str().filter(|_| !declares) {
                    if matches!(event["status"].as_str(), Some("pending" | "in_progress")) {
                        self.steps.insert(call.to_string());
                    } else {
                        self.steps.remove(call);
                    }
                }
            }
            "thought" | "agent" | "outcome" => self.heard = true,
            _ => {}
        }
    }
}

/// An `activity` line's words, or none for a kind said without any.
fn words(event: &Value) -> Option<String> {
    event["message"]
        .as_str()
        .map(str::trim)
        .filter(|words| !words.is_empty())
        .map(str::to_string)
}

impl Turn {
    /// Folds one line of the tape in, and says whether anything changed.
    fn apply(&mut self, event: &Value) -> bool {
        let before = self.clone();
        let kind = event["kind"].as_str().unwrap_or_default();
        match kind {
            // What waited behind the turn is still to be read in the next.
            "turn" => {
                let lines = self.next.take().map(|next| next.lines).unwrap_or_default();
                *self = Turn::default();
                self.ask.lines = lines;
            }
            "user" | "delivery" => {
                let Some(id) = event["id"].as_str() else {
                    return false;
                };
                let read = event["receipt"] == "read";
                if self.ask.lines.contains(id) {
                    self.ask.heard |= read;
                } else if self
                    .next
                    .as_ref()
                    .is_some_and(|next| next.lines.contains(id))
                {
                    if read {
                        self.ask = self.next.take().unwrap_or_default();
                        self.ask.heard = true;
                    }
                } else if read {
                    self.next = None;
                    self.ask = Ask {
                        lines: HashSet::from([id.to_string()]),
                        heard: true,
                        ..Ask::default()
                    };
                } else if !self.ask.heard {
                    // Nothing back yet: the agent takes it with the line before.
                    self.ask.lines.insert(id.to_string());
                } else {
                    self.next
                        .get_or_insert_default()
                        .lines
                        .insert(id.to_string());
                }
            }
            _ => {
                if kind == "activity" {
                    self.declared = words(event);
                }
                self.ask.apply(kind, event);
                if let Some(next) = &mut self.next {
                    next.apply(kind, event);
                }
            }
        }
        *self != before
    }
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

    /// A state read from a stretch of tape in the order it was written.
    pub fn of(tail: &[Value]) -> Self {
        let mut state = State::default();
        for event in tail {
            state.apply(event);
        }
        state
    }

    pub fn apply(&mut self, event: &Value) -> bool {
        let turned = self.turn.apply(event);
        if let Some(preview) = super::message_preview(event) {
            self.preview = serde_json::from_value(preview).ok();
            return true;
        }
        let kind = event["kind"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "permission"
                | "human_action"
                | "credential_ask"
                | "passkey_ask"
                | "secret_use_ask"
                | "cookie_ask"
                | "exchange_paused"
                | "quiet_paused"
        ) {
            return turned || kind == "delivery";
        }
        if let Some(id) = event["id"].as_str() {
            match kind {
                "permission" => self.set_thread_card(id, event.get("decision").is_none()),
                "human_action" | "credential_ask" | "cookie_ask" => {
                    self.set_thread_card(id, event["status"] == "pending");
                }
                _ => {}
            }
        }
        let Ok(event) = serde_json::from_value::<TranscriptEvent>(event.clone()) else {
            return true;
        };
        match event {
            TranscriptEvent::Permission { id, decision, .. } => {
                self.set_card(id, decision.is_none());
            }
            TranscriptEvent::HumanAction { id, status, .. } => {
                self.set_card(id, status == HumanActionStatus::Pending);
            }
            TranscriptEvent::CredentialAsk { id, status, .. } => {
                self.set_card(id, status == hotline_contract::CredentialStatus::Pending);
            }
            TranscriptEvent::CookieAsk { id, status, .. } => {
                self.set_card(id, status == hotline_contract::CookieAskStatus::Pending);
            }
            TranscriptEvent::SecretUseAsk { id, status, .. } => {
                self.set_card(id, status == hotline_contract::SecretUseStatus::Pending);
            }
            TranscriptEvent::PasskeyAsk { id, status, .. } => {
                self.set_card(id, status == PasskeyAskStatus::Pending);
            }
            TranscriptEvent::ExchangePaused { id, status, .. }
            | TranscriptEvent::QuietPaused { id, status, .. } => {
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
                // The turn is read in append order, not at the first position
                // of each id in the fold: a completed newer tool must not
                // resurrect an older overlapping one on restart, and a line
                // marked read is read where the mark was written. Only this
                // cold/lag seed reads a tail; live events keep order.
                state.turn = State::of(&super::tail(log.root(), persona_id)).turn;
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
        assert!(live.state.turn.ask.stepping(), "a is still running");
        log.append(&tape, &json!({"kind":"tool", "id":"a", "ts":2, "toolCallId":"a", "title":"a", "status":"completed"})).unwrap();
        live.refresh();
        assert!(!live.state.turn.ask.stepping());
        assert!(
            !Live::new(&Log::open(root.path()), tape)
                .state
                .turn
                .ask
                .stepping()
        );
    }

    fn line(kind: &str, id: &str, more: Value) -> Value {
        let mut line = json!({"kind":kind, "id":id, "ts":1});
        line.as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        line
    }

    /// A line sent while the agent is still on the last one waits: the ask
    /// it begins starts when the agent reads it, not when it was written.
    #[test]
    fn a_line_begins_its_ask_when_the_agent_reads_it() {
        let declared = line(
            "activity",
            "d",
            json!({"type":"reading", "message":"Reading the lease"}),
        );
        let state = State::of(&[
            line("user", "u1", json!({"text":"read it", "receipt":"read"})),
            declared.clone(),
            line(
                "user",
                "u2",
                json!({"text":"and the deposit", "receipt":"sent"}),
            ),
        ]);
        assert_eq!(
            state.turn.ask.declared.as_deref(),
            Some("Reading the lease")
        );
        let state = State::of(&[
            line("user", "u1", json!({"text":"read it", "receipt":"read"})),
            declared,
            line(
                "user",
                "u2",
                json!({"text":"and the deposit", "receipt":"read"}),
            ),
        ]);
        assert_eq!(state.turn.ask.declared, None);
        assert!(state.turn.ask.heard);
        // The turn's words stand through it: what a held turn still waits on.
        assert_eq!(state.turn.declared.as_deref(), Some("Reading the lease"));
    }

    /// The read mark lands after the agent's first update is heard, and a
    /// declaration writes its words before then: they are the new ask's,
    /// mid-turn or at a turn's start. A line queued behind a turn carries
    /// into the next one without what the old turn did meanwhile.
    #[test]
    fn what_comes_after_a_line_is_written_is_its_once_it_is_read() {
        let sent = |id: &str| line("user", id, json!({"text":"more", "receipt":"sent"}));
        let read = |id: &str| line("user", id, json!({"text":"more", "receipt":"read"}));
        let declared = line(
            "activity",
            "d",
            json!({"type":"searching", "message":"Waiting on the edge cases"}),
        );
        let thought = line("thought", "t", json!({"text":"hmm"}));
        for opening in [vec![], vec![read("u0"), thought.clone()]] {
            let mut tape = opening;
            tape.extend([sent("u1"), declared.clone(), read("u1")]);
            let state = State::of(&tape);
            assert_eq!(
                state.turn.ask.declared.as_deref(),
                Some("Waiting on the edge cases")
            );
        }
        let state = State::of(&[
            read("u0"),
            thought,
            sent("u1"),
            line(
                "activity",
                "old",
                json!({"type":"reading", "message":"Finishing the last one"}),
            ),
            line("turn", "e", json!({"stopReason":"end_turn"})),
            read("u1"),
        ]);
        assert_eq!(state.turn.ask.declared, None);
        assert!(state.turn.ask.heard);
    }

    /// Nothing back yet until the agent reads the line or says anything; a
    /// turn's end clears every fact of it.
    #[test]
    fn the_agent_is_heard_from_its_read_mark_or_its_first_line() {
        let sent = line("user", "u", json!({"text":"hi", "receipt":"sent"}));
        assert!(!State::of(std::slice::from_ref(&sent)).turn.ask.heard);
        let read = line("user", "u", json!({"text":"hi", "receipt":"read"}));
        assert!(State::of(&[sent.clone(), read]).turn.ask.heard);
        let thought = line("thought", "t", json!({"text":"hmm"}));
        assert!(State::of(&[sent.clone(), thought.clone()]).turn.ask.heard);
        let ended = State::of(&[
            sent,
            thought,
            line("turn", "e", json!({"stopReason":"end_turn"})),
        ]);
        assert_eq!(ended.turn, Turn::default());
    }

    /// The calls that declare are said, not done: they are no step.
    #[test]
    fn a_declaring_call_is_not_a_step() {
        let call = |status: &str| {
            line(
                "tool",
                "t",
                json!({"toolCallId":"t", "title":"activity", "status":status, "said":{"key":"activity", "one":"", "many":"", "doing":""}}),
            )
        };
        let state = State::of(&[call("in_progress")]);
        assert!(!state.turn.ask.stepping());
        assert!(state.turn.ask.heard);
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
