//! The one way an event reaches a thread.
//!
//! A conversation in the room is a thread whatever its kind (see
//! [`crate::thread`]), and what happens when something is said in one is the
//! same for all of them, bar what the kind's [`Policy`] decides. [`Threads::write`]
//! is that: it appends to the thread's stream, indexes what was said for
//! `search_thread`, and routes a card the agent raised. A card the person may
//! answer is pushed to their phone, signalled to a live call and counted in the
//! roster's `waiting`; one that nobody may answer is expired on the spot and
//! handed back, so the agent behind it is refused instead of left waiting.
//!
//! Sides and runs are written here. The DM's tape keeps its own door
//! ([`Room::write_value`]) until it is ported: chapters index it, and the turn
//! loop pushes its cards. Pairs and calls are not written through here yet.
//!
//! Unread is not kept here: a client counts it against each thread's newest
//! line, so a write's part is only to land one and to wake the roster row.

use super::{Room, now_ms};
use crate::contract::PermissionOption;
use crate::driver::Driver;
use crate::thread::{Answer, Policy, ThreadId, ThreadKind};
use serde::Serialize;
use serde_json::Value;

/// The write path, over one room.
pub(super) struct Threads<'a> {
    pub(super) room: &'a Room,
}

impl Room {
    pub(super) fn threads(&self) -> Threads<'_> {
        Threads { room: self }
    }
}

/// A card nobody may answer, which the write path expired. The agent that
/// raised it is still waiting on it until it is told no.
#[derive(Debug)]
pub(super) struct Refusal {
    pub request_id: String,
    /// The option that says no, when the card offered one.
    reject: Option<String>,
}

impl Refusal {
    /// Tells the agent no: the card's reject option when it has one, and
    /// otherwise stops the turn, which is the only other way out of the wait.
    pub(super) fn deliver(&self, driver: &dyn Driver) {
        match &self.reject {
            Some(option_id) if driver.answer_permission(&self.request_id, option_id) => {}
            _ => driver.cancel(),
        }
    }
}

/// What a write did beyond landing the line.
#[derive(Debug, Default)]
pub(super) struct Written {
    pub refused: Vec<Refusal>,
}

/// A card the agent raised and is waiting behind.
struct Card {
    request_id: String,
    title: String,
    options: Vec<PermissionOption>,
}

impl Card {
    /// The card an event raises, if it raises one. Only a permission is
    /// raised by a thread's own agent: the cards that park a tool on the
    /// person (`request_human`, a passkey) are written to the tape by the
    /// session that owns the wait.
    fn raised_by(event: &Value) -> Option<Self> {
        if event.get("kind").and_then(Value::as_str) != Some("permission")
            || event.get("decision").is_some()
        {
            return None;
        }
        Some(Self {
            request_id: event.get("requestId")?.as_str()?.to_string(),
            title: event.get("title")?.as_str()?.to_string(),
            options: serde_json::from_value(event.get("options")?.clone()).unwrap_or_default(),
        })
    }

    /// The option that refuses, by the kind the harness gave it.
    fn reject(&self) -> Option<String> {
        self.options
            .iter()
            .find(|option| {
                option
                    .kind
                    .as_deref()
                    .is_some_and(|kind| kind.starts_with("reject"))
            })
            .map(|option| option.option_id.clone())
    }
}

/// Whether an event is a card being raised or settled: either one changes
/// whether the thread is waiting on the person.
fn is_card(event: &Value) -> bool {
    event.get("kind").and_then(Value::as_str) == Some("permission")
}

impl Threads<'_> {
    /// Writes one event to a thread of `persona_id`'s. See the module.
    pub(super) fn write(
        &self,
        thread: &ThreadId,
        persona_id: &str,
        event: &impl Serialize,
    ) -> Written {
        let event = match serde_json::to_value(event) {
            Ok(event) => event,
            Err(error) => {
                eprintln!("an event for the {thread} could not be written: {error}");
                return Written::default();
            }
        };
        match thread.kind {
            ThreadKind::Dm => {
                self.room.write_value(persona_id, &event);
                return Written::default();
            }
            ThreadKind::Side | ThreadKind::Run => {}
            ThreadKind::Pair | ThreadKind::Call => {
                eprintln!("the {thread} is not written through the shared path yet");
                return Written::default();
            }
        }
        let Some(stream) = thread.stream() else {
            return Written::default();
        };
        if let Err(error) = self.room.log.append(&stream, &event) {
            eprintln!("the {thread} could not be written to: {error}");
            return Written::default();
        }
        let policy = Policy::of(thread.kind);
        if policy.surface.index {
            self.index(persona_id, thread, &event);
        }
        let mut written = Written::default();
        if !is_card(&event) {
            return written;
        }
        match (Card::raised_by(&event), policy.answer) {
            (Some(card), Answer::Nobody) => {
                let now = now_ms();
                for expired in crate::log::expire_orphaned_permissions(&[event], now) {
                    let _ = self.room.log.append(&stream, &expired);
                }
                written.refused.push(Refusal {
                    reject: card.reject(),
                    request_id: card.request_id,
                });
            }
            (Some(card), Answer::Person) => {
                if policy.surface.push_cards {
                    self.push(persona_id, thread, &card);
                }
                if policy.surface.mirror_to_call
                    && let Some(voice) = super::lock(&self.room.voice).upgrade()
                {
                    voice.card(persona_id, &event);
                }
                self.wake_roster(persona_id);
            }
            // A card settled: the thread may have stopped waiting.
            (None, Answer::Person) => self.wake_roster(persona_id),
            (None, Answer::Nobody) => {}
        }
        written
    }

    fn index(&self, persona_id: &str, thread: &ThreadId, event: &Value) {
        let mut indexer = super::lock(&self.room.indexer);
        let Some(indexer) = indexer.as_mut() else {
            return;
        };
        if let Err(error) = indexer.index_thread_event(persona_id, thread, event) {
            eprintln!("the search index rejected an event of the {thread}: {error}");
        }
    }

    fn push(&self, persona_id: &str, thread: &ThreadId, card: &Card) {
        let side_id = (thread.kind == ThreadKind::Side).then_some(thread.key.as_str());
        self.room.push.notify_in(
            side_id,
            &self.room.needs_you(persona_id),
            &card.title,
            persona_id,
            Some(crate::push::Waiting::Permission {
                request_id: card.request_id.clone(),
                options: card.options.clone(),
            }),
        );
    }

    /// The roster row recomputes `waiting` when the teammate's info changes.
    fn wake_roster(&self, persona_id: &str) {
        let _ = self.room.info_changes.send(self.room.info(persona_id));
    }
}

impl Room {
    /// Whether one of the teammate's threads other than its DM has a card
    /// waiting on the person, for the roster row.
    pub fn threads_waiting(&self, persona_id: &str) -> bool {
        self.side_cards_waiting(persona_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::SideEnd;
    use crate::driver::{MessageKind, Update};
    use crate::log::StreamId;
    use crate::session::runner::{RunEnd, RunSpec};
    use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
    use crate::thread::{AgentBinding, End, Participant, ThreadLink, ThreadState, ThreadStore};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    fn room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
        let log = scratch(name);
        enrol(&log, &persona("ada"));
        Room::with_agents_and_computers(
            log,
            Arc::new(DeskKeys),
            agents,
            crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
        )
    }

    fn said(id: &str, text: &str) -> Update {
        Update::Message {
            kind: MessageKind::Agent,
            id: id.to_string(),
            text: text.to_string(),
        }
    }

    fn turn() -> Update {
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }
    }

    fn card(option_kind: &str) -> Update {
        Update::Permission {
            request_id: "r1".to_string(),
            title: "Run the tests?".to_string(),
            options: vec![crate::contract::PermissionOption {
                option_id: "choice".to_string(),
                name: "Choice".to_string(),
                kind: Some(option_kind.to_string()),
            }],
        }
    }

    /// A phone paired with the desk, so a push has somewhere to go.
    fn pair_a_phone(room: &Room) {
        std::fs::write(
            room.log.root().join("remote.json"),
            json!({
                "desktopId": "desk-1",
                "host": "desk.local",
                "enabled": true,
                "grants": [{
                    "device": {"id": "phone-1", "name": "Phone", "pairedAt": 0},
                    "tokenHash": "hash",
                    "push": {"token": "ExponentPushToken[phone]", "platform": "ios"}
                }]
            })
            .to_string(),
        )
        .unwrap();
    }

    async fn until(mut done: impl FnMut() -> bool) {
        for _ in 0..500 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the room never got there");
    }

    fn spec(room: &Room, run_id: &str) -> RunSpec {
        RunSpec {
            persona_id: "ada".to_string(),
            run_id: run_id.to_string(),
            title: "Check the crane".to_string(),
            task: "Find out why the crane stopped.".to_string(),
            capability: room.capability_lease("ada"),
        }
    }

    // Phase 1: every stream loads as a thread.

    #[test]
    fn a_dm_is_the_teammate_with_the_session_its_checkpoint_holds() {
        let log = scratch("thread-dm");
        let mut ada = persona("ada");
        ada.session_checkpoints = vec![
            crate::contract::SessionCheckpoint {
                backend_id: "elsewhere".to_string(),
                session_id: "not-this-harness".to_string(),
            },
            crate::contract::SessionCheckpoint {
                backend_id: "hotline".to_string(),
                session_id: "s1".to_string(),
            },
        ];
        enrol(&log, &ada);
        let store = ThreadStore::new(&log);

        let dm = store.load(&ThreadId::dm("ada")).unwrap();
        assert_eq!(dm.kind(), ThreadKind::Dm);
        assert_eq!(dm.parent, None);
        assert_eq!(
            dm.participants,
            [Participant::Person, Participant::Persona("ada".into())]
        );
        assert_eq!(dm.state, ThreadState::Live);
        assert_eq!(
            dm.binding,
            Some(AgentBinding {
                backend_id: "hotline".into(),
                session_id: "s1".into()
            })
        );
        assert_eq!(store.load(&ThreadId::dm("nobody")), None);
        assert!(store.list("nobody").is_empty());
    }

    #[tokio::test]
    async fn a_side_thread_loads_from_its_marker_through_its_whole_life() {
        let agents = Fake::new(Scripted::new(vec![said("m1", "On it."), turn()]));
        agents.reporting("child-1", false);
        let room = room("thread-side", agents);
        let summary = room.start_side("ada", "Look at the winch").await.unwrap();
        let id = ThreadId::side(&summary.side_id);
        let store = ThreadStore::new(&room.log);
        until(|| room.side_threads("ada").iter().all(|side| !side.working)).await;

        let side = store.load(&id).unwrap();
        assert_eq!(side.state, ThreadState::Live);
        assert_eq!(side.title.as_deref(), Some("Look at the winch"));
        assert_eq!(
            side.parent,
            Some(ThreadLink {
                thread: ThreadId::dm("ada"),
                event: Some(format!("link:side:{}", summary.side_id)),
            })
        );
        assert_eq!(
            side.participants,
            [Participant::Person, Participant::Persona("ada".into())]
        );
        assert_eq!(
            side.binding,
            Some(AgentBinding {
                backend_id: "hotline".into(),
                session_id: "child-1".into()
            }),
            "the saved session is the thread's binding"
        );

        // The teammate lists its DM and then the thread.
        let listed: Vec<ThreadId> = store.list("ada").into_iter().map(|t| t.id).collect();
        assert_eq!(listed, [ThreadId::dm("ada"), id.clone()]);

        room.archive_side(&summary.side_id, SideEnd::Person, None)
            .unwrap();
        assert_eq!(
            store.load(&id).unwrap().state,
            ThreadState::Closed(End::Person)
        );
    }

    #[tokio::test]
    async fn a_run_loads_from_its_marker_and_finds_its_teammate_on_the_tape() {
        let agents = Fake::new(Scripted::new(vec![said("m1", "Jammed."), turn()]));
        let room = room("thread-run", agents);
        let outcome = room.run(spec(&room, "r1"), CancellationToken::new()).await;
        assert_eq!(outcome.end, RunEnd::Done);
        let store = ThreadStore::new(&room.log);

        let run = store.load(&ThreadId::run("r1")).unwrap();
        assert_eq!(run.state, ThreadState::Closed(End::Done));
        assert_eq!(run.title.as_deref(), Some("Check the crane"));
        assert_eq!(run.participants, [Participant::Persona("ada".into())]);
        assert_eq!(
            run.parent,
            Some(ThreadLink {
                thread: ThreadId::dm("ada"),
                event: Some("link:run:r1".into()),
            })
        );
        assert_eq!(store.load(&ThreadId::run("elsewhere")), None);
        assert!(store.list("ada").iter().any(|thread| thread.id == run.id));
    }

    #[test]
    fn a_pair_loads_from_its_sidecar_and_is_live_while_a_request_is_answered() {
        let log = scratch("thread-pair");
        enrol(&log, &persona("ada"));
        enrol(&log, &persona("bob"));
        let key = crate::paths::thread_key("ada", "bob").unwrap();
        crate::log::thread::ensure(log.root(), &key).unwrap();
        let store = ThreadStore::new(&log);

        let pair = store.load(&ThreadId::pair(&key)).unwrap();
        assert_eq!(pair.state, ThreadState::Parked);
        assert_eq!(
            pair.participants,
            [
                Participant::Persona("ada".into()),
                Participant::Persona("bob".into())
            ]
        );
        assert_eq!(store.load(&ThreadId::pair("ada\u{1f}zed")), None);

        log.append(
            &StreamId::Room,
            &json!({
                "kind": "exchange_pair", "id": key, "a": "ada", "b": "bob",
                "exchanges": 0, "paused": false,
                "requests": [{
                    "id": "q1", "from": "ada", "to": "bob", "message": "hi",
                    "intent": "ask", "phase": "running", "reply": "", "failed": false
                }]
            }),
        )
        .unwrap();
        assert_eq!(
            store.load(&ThreadId::pair(&key)).unwrap().state,
            ThreadState::Live
        );
    }

    #[tokio::test]
    async fn every_stream_in_the_room_is_listed_once() {
        let agents = Fake::new(Scripted::new(vec![said("m1", "Jammed."), turn()]));
        let room = room("thread-all", agents);
        enrol(&room.log, &persona("bob"));
        let key = crate::paths::thread_key("ada", "bob").unwrap();
        crate::log::thread::ensure(room.log.root(), &key).unwrap();
        let side = room.start_side("ada", "Winch").await.unwrap();
        room.run(spec(&room, "r1"), CancellationToken::new()).await;

        let ids: Vec<String> = ThreadStore::new(&room.log)
            .all()
            .into_iter()
            .map(|thread| thread.id.to_string())
            .collect();
        let mut expected = vec![
            "dm:ada".to_string(),
            format!("side:{}", side.side_id),
            "run:r1".to_string(),
            format!("pair:{key}"),
            "dm:bob".to_string(),
        ];
        let mut found = ids.clone();
        expected.sort();
        found.sort();
        assert_eq!(found, expected, "each stream once, the pair not twice");
    }

    // Phase 2: one write path.

    #[tokio::test]
    async fn a_permission_card_in_a_side_thread_pushes_to_the_phone_and_sets_waiting() {
        let gate = Arc::new(Semaphore::new(1));
        let agents = Fake::new(Scripted::new(vec![card("allow_once"), turn()]).gated(gate));
        agents.awaiting("r1");
        let room = room("thread-side-card", agents);
        pair_a_phone(&room);
        let mut info = room.subscribe_info();
        let summary = room.start_side("ada", "Task").await.unwrap();
        let stream = StreamId::Side(summary.side_id.clone());
        until(|| {
            room.log
                .load(&stream)
                .iter()
                .any(|event| event["kind"] == "permission")
        })
        .await;

        assert!(room.threads_waiting("ada"), "the roster row says it waits");
        let mut woke = false;
        while let Ok(change) = info.try_recv() {
            woke |= change.persona_id == "ada";
        }
        assert!(woke, "the row is told to read itself again");
        let sent = room.sent_voice_pushes();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["title"], "Ada needs you");
        assert_eq!(sent[0]["body"], "Run the tests?");
        assert_eq!(sent[0]["categoryId"], "permission");
        assert_eq!(sent[0]["data"]["requestId"], "r1");
        assert_eq!(
            sent[0]["data"]["sideId"],
            summary.side_id.as_str(),
            "the phone answers in the thread, not the DM"
        );

        room.answer_side_permission(&summary.side_id, "r1", "choice")
            .await
            .unwrap();
        assert!(!room.threads_waiting("ada"), "answered, so not waiting");
    }

    #[tokio::test]
    async fn a_card_raised_in_a_run_expires_and_its_agent_is_told_no() {
        let agents = Fake::new(Scripted::new(vec![card("reject_once"), turn()]));
        agents.awaiting("r1");
        let room = room("thread-run-card", agents.clone());
        pair_a_phone(&room);

        let outcome = room.run(spec(&room, "r1"), CancellationToken::new()).await;
        assert_eq!(outcome.end, RunEnd::Done);
        let stream = room.log.load(&StreamId::Run("r1".into()));
        let card = stream
            .iter()
            .find(|event| event["kind"] == "permission")
            .unwrap();
        assert_eq!(card["decision"], "expired");
        assert!(
            agents.waiting().is_empty(),
            "the agent was answered with the card's reject option"
        );
        assert!(!room.threads_waiting("ada"), "nobody is waited on");
        assert!(room.sent_voice_pushes().is_empty(), "nothing to push");
    }

    #[tokio::test]
    async fn a_run_card_with_no_way_to_say_no_stops_the_turn() {
        // A run ending cancels its driver once whatever happened in it.
        let plain = Fake::new(Scripted::new(vec![said("m1", "Done."), turn()]));
        let quiet = room("thread-run-plain", plain.clone());
        quiet
            .run(spec(&quiet, "r1"), CancellationToken::new())
            .await;

        let agents = Fake::new(Scripted::new(vec![card("allow_once"), turn()]));
        agents.awaiting("r1");
        let room = room("thread-run-card-cancel", agents.clone());
        room.run(spec(&room, "r1"), CancellationToken::new()).await;
        assert_eq!(agents.cancel_count(), plain.cancel_count() + 1);
    }

    #[tokio::test]
    async fn what_was_said_in_a_side_thread_is_found_naming_it_and_not_by_the_window() {
        let agents = Fake::new(Scripted::new(vec![
            said("m1", "The winch is jammed."),
            turn(),
        ]));
        let room = room("thread-search-side", agents);
        let summary = room.start_side("ada", "Task").await.unwrap();
        until(|| room.side_threads("ada").iter().all(|side| !side.working)).await;

        let root = room.log.root();
        let found = crate::store::search::search_teammate(root, "ada", "winch", None).unwrap();
        assert_eq!(
            found["hits"][0]["thread"],
            format!("side:{}", summary.side_id)
        );
        let tape_only = crate::store::search::search(root, "ada", "winch", None).unwrap();
        assert!(
            tape_only["hits"].as_array().unwrap().is_empty(),
            "the window's own search stays on the tape: {tape_only}"
        );
    }

    #[tokio::test]
    async fn what_a_run_said_is_found_naming_the_run() {
        let agents = Fake::new(Scripted::new(vec![
            said("m1", "The winch is oiled."),
            turn(),
        ]));
        let room = room("thread-search-run", agents);
        room.run(spec(&room, "r1"), CancellationToken::new()).await;

        let found =
            crate::store::search::search_teammate(room.log.root(), "ada", "winch", None).unwrap();
        assert_eq!(found["hits"][0]["thread"], "run:r1");
    }
}
