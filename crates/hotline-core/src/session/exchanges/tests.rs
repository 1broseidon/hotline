mod ending;
mod files;
mod handed_over;
mod lasting;
mod loop_in;
mod newer;
mod pause;
mod stops;
mod threads;

use super::*;
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use crate::test_support::until;
use std::time::Duration;

fn setup(name: &str) -> (Arc<Room>, Arc<Fake>) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let agents = answering();
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}
/// Agents whose every turn answers `result-<n>`, the nth turn they take.
fn answering() -> Arc<Fake> {
    let turns = (0..50)
        .map(|i| {
            vec![
                Update::Message {
                    kind: MessageKind::Agent,
                    id: format!("answer-{i}"),
                    text: format!("result-{i}"),
                },
                Update::Turn {
                    stop_reason: "end_turn".into(),
                    usage: None,
                },
            ]
        })
        .collect();
    Fake::new(Scripted::turns(turns))
}
/// Ada sends Bob an item that wants work, and Bob takes it into a work
/// thread at once: answers the request the work runs as. Every request is
/// work taken from the inbox.
async fn send(room: &Arc<Room>, intent: &str) -> String {
    assert_eq!(intent, "handoff", "nothing else is a request now");
    let sent = TeammateTools::new(room, "ada")
        .call(
            "send",
            &json!({"to":["bob"],"title":"work handoff","message":"work handoff","wants":"work"}),
        )
        .await
        .unwrap();
    let item = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    TeammateTools::new(room, "bob")
        .call(
            "update_inbox",
            &json!({"items":[{"id": item, "state": "taken"}]}),
        )
        .await
        .unwrap();
    crate::store::inbox::of(&room.log, "bob")
        .into_iter()
        .find(|kept| kept.id == item)
        .and_then(|kept| kept.request)
        .expect("taking the item made a request")
}
/// What came of request `id` in Ada's inbox, shaped as its delivery was:
/// the reply's words as `text`, and `cause.status` done or failed.
fn result(room: &Room, id: &str) -> Option<serde_json::Value> {
    let request = room
        .exchange_pairs()
        .into_iter()
        .flat_map(|pair| pair.requests)
        .find(|r| r.id == id)?;
    let reply = crate::store::inbox::of(&room.log, &request.from)
        .into_iter()
        .find(|reply| match request.items.first() {
            Some(item) => reply.reply_to.as_deref() == Some(item.as_str()),
            None => reply.reply_to.is_none() && reply.from == request.to,
        })?;
    Some(json!({
        "text": reply.body,
        "cause": {
            "requestId": id,
            "status": if request.failed() { "failed" } else { "done" },
        },
    }))
}
/// One scripted turn: what the teammate says, and how the turn ends.
pub(super) fn turn(id: &str, text: &str, ends: &str) -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: id.into(),
            text: text.into(),
        },
        Update::Turn {
            stop_reason: ends.into(),
            usage: None,
        },
    ]
}
pub(super) fn request(room: &Room, key: &str, id: &str) -> Request {
    room.exchange_pair(key)
        .and_then(|pair| pair.requests.into_iter().find(|r| r.id == id))
        .expect("the request is saved")
}
/// What came back to Ada of the request `id`.
pub(super) fn answer_to(room: &Room, id: &str) -> Option<serde_json::Value> {
    result(room, id)
}
async fn done(room: &Room, id: &str) {
    until("the request is done", || {
        room.exchange_pair("ada~bob").is_some_and(|p| {
            p.requests
                .iter()
                .any(|r| r.id == id && r.phase() == Phase::Done)
        })
    })
    .await;
}
/// The work thread a handoff runs in, once it has one.
fn work_thread(room: &Room, id: &str) -> String {
    room.exchange_pair("ada~bob")
        .and_then(|p| {
            p.requests
                .iter()
                .find(|r| r.id == id)
                .and_then(|r| r.thread.clone())
        })
        .expect("the handoff opened a work thread")
}
fn saved_request(id: &str, intent: Intent, phase: Phase) -> Request {
    Request {
        id: id.into(),
        from: "ada".into(),
        to: "bob".into(),
        message: "saved work".into(),
        intent,
        state: State::of(phase),
        reply: String::new(),
        answer: String::new(),
        request_counted: false,
        human_actions: vec![],
        thread: None,
        reply_thread: None,
        reply_job: None,
        lead_call: None,
        released: false,
        about: None,
        own_thread: false,
        looped_in: false,
        files: Vec::new(),
        items: Vec::new(),
    }
}
fn seed(room: &Room, request: Request, count: i64, paused: bool) {
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: count,
        paused,
        stopped: false,
        requests: vec![request],
        newer: Newer::default(),
    })
    .unwrap();
}

#[tokio::test]
async fn taken_work_runs_in_its_own_thread_and_returns_the_matching_result_to_the_senders_inbox() {
    let (room, agents) = setup("handoff-context-result");
    room.write_value(
        "bob",
        &json!({"kind":"user","id":"secret","ts":1,"text":"recipient main context"}),
    );
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    done(&room, &id).await;
    let thread = work_thread(&room, &id);
    // The brief lands in the work thread, with its provenance, not in bob's DM.
    let stream = room.log.load(&StreamId::Side(thread.clone()));
    let brief = stream
        .iter()
        .find(|v| v["cause"]["kind"] == "handoff")
        .expect("the brief is in the thread");
    assert_eq!(brief["cause"]["requestId"], id);
    assert_eq!(brief["from"]["kind"], "dm");
    assert_eq!(brief["from"]["thread"], "ada");
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["cause"]["kind"] != "handoff"),
        "a handoff never lands in the target's main conversation"
    );
    // The result returns to the sender's inbox, from the work thread.
    let answer = result(&room, &id).unwrap();
    assert_eq!(answer["text"], "result-0");
    assert_eq!(answer["cause"]["status"], "done");
    // The thread has the target's context, and both DMs carry a link marker.
    let preambles = lock(&agents.preambles).clone();
    let preamble = preambles
        .iter()
        .find(|p| p.contains("This is a work thread"))
        .expect("the thread has the work brief");
    assert!(preamble.contains("recipient main context"), "{preamble}");
    for owner in ["ada", "bob"] {
        assert!(
            room.tape(owner)
                .iter()
                .any(|v| v["kind"] == "link" && v["thread"] == thread.as_str()),
            "{owner} has a marker: {:?}",
            room.tape(owner)
        );
    }
    assert!(
        room.log.load(&StreamId::Pair("ada~bob".into())).is_empty(),
        "no thread between the two of them is written"
    );
}

#[tokio::test]
async fn aborted_or_revoked_handoff_turns_return_failure_not_success() {
    for reason in ["aborted", "revoked"] {
        let log = scratch(&format!("handoff-{reason}"));
        enrol(&log, &persona("ada"));
        enrol(&log, &persona("bob"));
        let agents = Fake::new(Scripted::new(vec![Update::Turn {
            stop_reason: reason.into(),
            usage: None,
        }]));
        let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
        room.allow_sender("bob", "ada").unwrap();
        let id = send(&room, "handoff").await;
        done(&room, &id).await;
        let answer = result(&room, &id).unwrap();
        assert_eq!(answer["cause"]["status"], "failed", "{reason}");
        assert!(
            answer["text"]
                .as_str()
                .unwrap()
                .contains("inspect before retrying")
        );
    }
}

#[tokio::test]
async fn twelve_counts_items_and_results_and_a_paused_pair_sends_nothing() {
    let (room, _) = setup("cross-intent-brake");
    room.allow_sender("bob", "ada").unwrap();
    for _ in 0..6 {
        let id = send(&room, "handoff").await;
        done(&room, &id).await;
    }
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.exchanges, 12, "six items and six results");
    assert!(pair.paused);
    let refused = TeammateTools::new(&room, "ada")
        .call("send", &json!({"to":["bob"],"title":"one more"}))
        .await
        .unwrap_err();
    assert!(refused.contains("paused at 12"), "{refused}");
    room.resume_exchange("ada", "bob").unwrap();
    let queued = send(&room, "handoff").await;
    done(&room, &queued).await;
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 2);
    room.heard_from_person("bob");
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 0);
}

#[tokio::test]
async fn the_twelfth_request_keeps_its_reply_queued_until_keep_going() {
    let (room, _) = setup("brake-reply");
    room.allow_sender("bob", "ada").unwrap();
    seed(
        &room,
        saved_request("twelfth", Intent::Handoff, Phase::Queued),
        11,
        false,
    );
    room.recover_queued_exchanges();
    until("the ada~bob request is at Reply", || {
        room.exchange_pair("ada~bob").unwrap().requests[0].phase() == Phase::Reply
    })
    .await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert!(pair.paused);
    assert_eq!(pair.exchanges, 12);
    assert_ne!(pair.requests[0].reply, "");
    assert!(result(&room, "twelfth").is_none(), "its result waits");
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, "twelfth").await;
    assert!(result(&room, "twelfth").is_some());
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 1);
}

#[tokio::test]
async fn restart_preserves_a_paused_queue_and_unstarted_handoff_without_counting_twice() {
    let (old, _) = setup("restart-paused-exchange");
    old.allow_sender("bob", "ada").unwrap();
    let mut request = saved_request("waiting", Intent::Handoff, Phase::Running);
    request.request_counted = true;
    seed(&old, request, 12, true);
    let log = old.log.clone();
    drop(old);
    let agents = Fake::new(Scripted::new(vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "a".into(),
            text: "recovered result".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_queued_exchanges();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts(), [] as [String; 0]);
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase(),
        Phase::Queued
    );
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, "waiting").await;
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().exchanges,
        1,
        "only the reply is new"
    );
}

#[tokio::test]
async fn restart_never_replays_started_work_and_delivers_an_explicit_failure() {
    let (room, agents) = setup("restart-started-exchange");
    let mut request = saved_request("started", Intent::Handoff, Phase::Running);
    request.state = State::Running { begun: true };
    request.request_counted = true;
    seed(&room, request, 1, false);
    room.reconcile_exchanges();
    room.recover_queued_exchanges();
    done(&room, "started").await;
    until("only the sender hears the failure", || {
        agents.prompts().len() == 1
    })
    .await;
    assert_eq!(room.tape("bob"), [] as [serde_json::Value; 0]);
    let result = result(&room, "started").unwrap();
    assert_eq!(result["cause"]["status"], "failed");
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("inspect before retrying")
    );
}

#[tokio::test]
async fn stop_and_revocation_settle_saved_work_and_resume_cannot_resurrect_it() {
    let (room, agents) = setup("stop-revoke-exchange");
    seed(
        &room,
        saved_request("stopped", Intent::Ask, Phase::Queued),
        12,
        true,
    );
    room.stop_exchange("ada", "bob").unwrap();
    room.resume_exchange("ada", "bob").unwrap();
    room.recover_queued_exchanges();
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase(),
        Phase::Stopped
    );
    seed(
        &room,
        saved_request("revoked", Intent::Handoff, Phase::Queued),
        12,
        true,
    );
    room.invalidate("bob").unwrap();
    room.resume_exchange("ada", "bob").unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase(),
        Phase::Stopped
    );
    assert_eq!(agents.prompts(), [] as [String; 0]);
    assert!(
        room.tape("ada")
            .iter()
            .any(|v| v["id"] == "exchange-ended:revoked")
    );
}

/// Revoking a teammate stops everything waiting on it at once; the sender is
/// told in one line that names the teammate, not one line per message with
/// its id.
#[tokio::test]
async fn a_revoke_tells_the_sender_once_by_name_however_many_it_stopped() {
    let (room, _) = setup("revoke-one-line");
    let mut requests = Vec::new();
    for (id, message) in [
        ("one", "first ask"),
        ("two", "second ask"),
        ("three", "third ask"),
    ] {
        let mut request = saved_request(id, Intent::Ask, Phase::Queued);
        request.message = message.into();
        requests.push(request);
    }
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: 0,
        paused: false,
        stopped: false,
        requests,
        newer: Newer::default(),
    })
    .unwrap();
    room.invalidate("bob").unwrap();
    let ended: Vec<serde_json::Value> = room
        .tape("ada")
        .into_iter()
        .filter(|v| {
            v["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("exchange-ended:"))
        })
        .collect();
    assert_eq!(ended.len(), 1, "{ended:?}");
    let name = room.persona("bob").unwrap().name;
    let text = ended[0]["text"].as_str().unwrap();
    assert!(
        text.starts_with(&format!(
            "3 messages to {name} stopped, the last (third ask)"
        )),
        "{text}"
    );
    assert!(
        !text.contains("bob ("),
        "names the teammate, not its id: {text}"
    );
}

#[tokio::test]
async fn a_legacy_grant_requires_informed_approval_for_work_but_still_lets_a_question_through() {
    let (room, _) = setup("legacy-handoff-grant");
    // Ada with her own folders asks Bob, with the whole machine: up in access.
    let mut caller = room.persona("ada").unwrap();
    caller.reach = Some(crate::contract::Reach::Workspace);
    crate::room::append_persona(&room.log, &caller).unwrap();
    let mut target = room.persona("bob").unwrap();
    target.allowed_senders.push("ada".into());
    crate::room::append_persona(&room.log, &target).unwrap();
    let held = |sent: &str| {
        let id = serde_json::from_str::<serde_json::Value>(sent).unwrap()["sent"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        crate::store::inbox::of(&room.log, "bob")
            .into_iter()
            .find(|item| item.id == id)
            .unwrap()
            .held
    };
    let tools = TeammateTools::new(&room, "ada");
    let question = tools
        .call("send", &json!({"to":["bob"],"title":"a question"}))
        .await
        .unwrap();
    assert!(!held(&question), "a question needs no informed approval");
    let work = tools
        .call(
            "send",
            &json!({"to":["bob"],"title":"some work","wants":"work"}),
        )
        .await
        .unwrap();
    assert!(held(&work), "work waits for the card");
    let card = || {
        room.tape("ada")
            .into_iter()
            .find(|v| v["kind"] == "permission" && v.get("decision").is_none())
    };
    until("an open permission card on ada's tape", || card().is_some()).await;
    assert!(
        card().unwrap()["title"]
            .as_str()
            .unwrap()
            .contains(" to do work\n\n")
    );
    room.answer_permission(
        "ada",
        card().unwrap()["requestId"].as_str().unwrap(),
        "allow_always",
    )
    .await
    .unwrap();
    until("the work lands", || !held(&work)).await;
    let next = tools
        .call(
            "send",
            &json!({"to":["bob"],"title":"more work","wants":"work"}),
        )
        .await
        .unwrap();
    assert!(!held(&next));
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|v| v["kind"] == "permission")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_handoff_runs_beside_the_persons_turn_and_stop_leaves_that_turn_alone() {
    let log = scratch("handoff-turn-boundary");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let agents = Fake::new(
        Scripted::new(vec![Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        }])
        .gated(gate.clone()),
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.allow_sender("bob", "ada").unwrap();
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("the agents have one prompt", || agents.prompts().len() == 1).await;
    let id = send(&room, "handoff").await;
    // It does not wait for the person's turn: it has a thread of its own.
    until("the agents have two prompts", || {
        agents.prompts().len() == 2
    })
    .await;
    assert!(room.mid_turn("bob"), "the person's turn is still going");
    let thread = work_thread(&room, &id);
    assert_eq!(room.sides("bob").len(), 1);
    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase(),
        Phase::Stopped
    );
    assert!(room.sides("bob").is_empty(), "{thread} closed");
    assert!(
        room.mid_turn("bob"),
        "stopping the handoff is not stopping the person"
    );
    gate.add_permits(10);
    until("bob's turn ends", || !room.mid_turn("bob")).await;
}

#[tokio::test]
async fn concurrent_handoffs_keep_distinct_result_correlations() {
    let (room, _) = setup("handoff-distinct-results");
    room.allow_sender("bob", "ada").unwrap();
    let (one, two) = tokio::join!(send(&room, "handoff"), send(&room, "handoff"));
    assert_ne!(one, two);
    done(&room, &one).await;
    done(&room, &two).await;
    for id in [one, two] {
        let pair = room.exchange_pair("ada~bob").unwrap();
        assert_eq!(
            result(&room, &id).unwrap()["text"],
            pair.requests.iter().find(|r| r.id == id).unwrap().reply
        );
    }
}

// Separate drivers matter here: cancelling a peer must not cancel a person's
// turn on another teammate, something the shared-driver Fake cannot prove.
use crate::contract::{Attachment, Persona, Reach};
use crate::driver::{Driver, DriverInfo};
use crate::session::Agents;
use std::collections::HashMap;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

struct ControlledDriver {
    script: Scripted,
    startup: Arc<Semaphore>,
    updates: Arc<Semaphore>,
    starts: AtomicUsize,
    prompts: AtomicUsize,
}
impl ControlledDriver {
    fn new() -> Arc<Self> {
        let updates = Arc::new(Semaphore::new(0));
        Arc::new(Self {
            script: Scripted::new(vec![
                Update::Message {
                    kind: MessageKind::Agent,
                    id: "answer".into(),
                    text: "true result".into(),
                },
                Update::Turn {
                    stop_reason: "end_turn".into(),
                    usage: None,
                },
            ])
            .gated(updates.clone()),
            startup: Arc::new(Semaphore::new(100)),
            updates,
            starts: AtomicUsize::new(0),
            prompts: AtomicUsize::new(0),
        })
    }
    fn prompts(&self) -> usize {
        self.prompts.load(Ordering::SeqCst)
    }
}
#[async_trait::async_trait]
impl Driver for ControlledDriver {
    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.startup.acquire().await.unwrap().forget();
        self.script.start(persona).await
    }
    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        self.prompts.fetch_add(1, Ordering::SeqCst);
        self.script.prompt(text, attachments, reach).await
    }
    fn cancel(&self) {
        self.script.cancel();
    }
    async fn set_model(&self, model: &str) -> Result<DriverInfo, String> {
        self.script.set_model(model).await
    }
}
struct ControlledAgents {
    drivers: HashMap<String, Arc<ControlledDriver>>,
    tools: Mutex<HashMap<String, TeammateTools>>,
    seeds: Mutex<Vec<Vec<crate::driver::rig::Said>>>,
}
#[async_trait::async_trait]
impl Agents for ControlledAgents {
    fn agent(
        &self,
        persona: &Persona,
        _preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        _mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        lock(&self.seeds).push(said);
        lock(&self.tools).insert(persona.id.clone(), tools);
        Ok(self.drivers[&persona.id].clone())
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}
fn controlled(name: &str) -> (Arc<Room>, Arc<ControlledAgents>) {
    let log = scratch(name);
    let mut drivers = HashMap::new();
    for id in ["ada", "bob", "cara"] {
        enrol(&log, &persona(id));
        drivers.insert(id.into(), ControlledDriver::new());
    }
    let agents = Arc::new(ControlledAgents {
        drivers,
        tools: Mutex::new(HashMap::new()),
        seeds: Mutex::new(Vec::new()),
    });
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}

#[tokio::test]
async fn delayed_boot_recovery_preserves_a_new_long_running_handoff_and_its_real_result() {
    let (room, agents) = controlled("live-during-boot-recovery");
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    until("bob is prompted", || agents.drivers["bob"].prompts() == 1).await;
    // Cross the real startup recovery deadline, not a substitute recovery path.
    tokio::time::sleep(Duration::from_millis(5300)).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.requests[0].phase(), Phase::Running);
    assert!(pair.requests[0].begun());
    agents.drivers["bob"].updates.add_permits(2);
    done(&room, &id).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.requests[0].reply, "true result");
    assert!(!pair.requests[0].failed());
    agents.drivers["ada"].updates.add_permits(2);
}

#[tokio::test]
async fn turn_admission_refuses_expired_dependency_before_the_worker_observes_it() {
    let (room, _) = setup("handoff-admission-lease");
    let origin = room.capability_lease("ada");
    let delegated = origin.scoped();
    seed(
        &room,
        saved_request("expired-at-admission", Intent::Handoff, Phase::Running),
        1,
        false,
    );
    lock(&room.exchange_leases).insert(
        "expired-at-admission".into(),
        (delegated, room.capability_lease("bob")),
    );
    origin.revoke();
    assert!(room.begin_request("expired-at-admission").is_err());
    assert!(!room.exchange_pair("ada~bob").unwrap().requests[0].begun());
}

/// Drive the real handoff and request_human handlers, stopping only the fake
/// model's updates so the tool call happens inside its actual handoff turn.
async fn suspended_handoff(name: &str) -> (Arc<Room>, String, String) {
    let (room, id, action, gate) = handoff_asking(name).await;
    gate.add_permits(2);
    until("the request waits on a person and bob's turn ends", || {
        room.exchange_pair("ada~bob").unwrap().requests[0].phase() == Phase::WaitingHuman
            && !room.mid_turn("bob")
    })
    .await;
    assert!(
        result(&room, &id).is_none(),
        "waiting is not a final result"
    );
    (room, id, action)
}

/// A handoff whose turn has asked the person and is still running: the
/// returned gate lets the fake model finish it.
async fn handoff_asking(name: &str) -> (Arc<Room>, String, String, Arc<tokio::sync::Semaphore>) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let agents = Fake::new(
        Scripted::new(vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "waiting".into(),
                text: "Waiting for the person".into(),
            },
            Update::Turn {
                stop_reason: "end_turn".into(),
                usage: None,
            },
        ])
        .gated(gate.clone()),
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    until("the agents have one prompt", || agents.prompts().len() == 1).await;
    // The handoff runs in its own work thread; its tools are that thread's.
    let thread = work_thread(&room, &id);
    TeammateTools::new(&room, "bob")
        .for_work(thread.clone())
        .call("request_human", &json!({"reason":"Approve the deployment"}))
        .await
        .unwrap();
    let action = room
        .log
        .load(&StreamId::Side(thread))
        .iter()
        .find(|v| v["kind"] == "human_action")
        .unwrap()["actionId"]
        .as_str()
        .unwrap()
        .to_string();
    (room, id, action, gate)
}

/// A card withdrawn while the handoff's turn runs no longer holds the
/// result back: the turn's end returns it. Once the turn has ended to wait
/// on a card, only an answer carries the work on, so that card stays.
#[tokio::test]
async fn a_withdrawn_card_lets_a_running_handoff_finish_and_a_waiting_one_keeps_its_card() {
    let (room, id, action, gate) = handoff_asking("human-handoff-withdraw").await;
    room.withdraw_human("bob", &action).unwrap();
    gate.add_permits(2);
    until("the handoff's result reaches ada", || {
        result(&room, &id).is_some()
    })
    .await;

    let (room, _id, action) = suspended_handoff("human-handoff-withdraw-waiting").await;
    let refused = room.withdraw_human("bob", &action);
    assert!(refused.is_err(), "{refused:?}");
    assert!(matches!(
        room.human_card("bob", &action),
        Some((
            TranscriptEvent::HumanAction {
                status: crate::contract::HumanActionStatus::Pending,
                ..
            },
            _
        ))
    ));
}

fn restart_human_room(old: Arc<Room>) -> (Arc<Room>, Arc<Fake>) {
    let log = old.log.clone();
    drop(old);
    let agents = Fake::new(Scripted::new(vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "finished".into(),
            text: "Finished after the human answer".into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]));
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}

#[tokio::test]
async fn a_human_gated_handoff_routes_its_final_result_after_restart_for_done_and_declined() {
    for answer in [
        crate::contract::HumanAnswer::Done,
        crate::contract::HumanAnswer::Declined,
    ] {
        let (old, id, action) = suspended_handoff("human-handoff-restart").await;
        let (room, agents) = restart_human_room(old);
        room.recover_exchanges().await;
        assert!(
            agents.prompts().is_empty(),
            "no old work is replayed while waiting"
        );
        room.answer_human("bob", &action, answer, Some("operator note".into()))
            .unwrap();
        done(&room, &id).await;
        until("the agents have two prompts", || {
            agents.prompts().len() == 2
        })
        .await;
        let answered = result(&room, &id).unwrap();
        assert_eq!(answered["text"], "Finished after the human answer");
        assert_eq!(answered["cause"]["status"], "done");
        assert!(agents.prompts()[0].contains("operator note"));
        assert!(!agents.prompts()[0].contains("work handoff"));
        room.recover_exchanges().await;
        assert_eq!(
            crate::store::inbox::of(&room.log, "ada")
                .iter()
                .filter(|reply| reply.reply_to.is_some())
                .count(),
            1
        );
        assert!(
            room.answer_human("bob", &action, crate::contract::HumanAnswer::Done, None)
                .is_err()
        );
        assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 2);
    }
}

#[tokio::test]
async fn a_saved_human_answer_recovers_the_gap_before_delivery() {
    for paused in [false, true] {
        let (old, id, action) = suspended_handoff("human-answer-dispatch-gap").await;
        if paused {
            let mut pair = old.exchange_pair("ada~bob").unwrap();
            pair.exchanges = EXCHANGE_CAP;
            pair.paused = true;
            old.save_pair(&pair).unwrap();
        }
        old.supersede_human(
            "bob",
            &action,
            crate::contract::HumanActionStatus::Done,
            Some("saved answer".into()),
        );
        let (room, agents) = restart_human_room(old);
        room.recover_exchanges().await;
        if paused {
            until("the ada~bob request is at Reply", || {
                room.exchange_pair("ada~bob").unwrap().requests[0].phase() == Phase::Reply
            })
            .await;
            assert_eq!(
                agents.prompts().len(),
                1,
                "the existing work continues, but its automatic reply waits at the cap"
            );
            room.resume_exchange("ada", "bob").unwrap();
        }
        done(&room, &id).await;
        until("the agents have two prompts", || {
            agents.prompts().len() == 2
        })
        .await;
        assert!(agents.prompts()[0].contains("saved answer"));
    }
}

/// The answer reached the work thread's stream, and the desk stopped before a
/// turn was admitted for it: it is still `sent`, and recovery hands it over.
#[tokio::test]
async fn a_human_answer_saved_to_the_thread_but_never_admitted_is_delivered_after_restart() {
    let (old, id, action) = suspended_handoff("human-answer-saved-unadmitted").await;
    let thread = work_thread(&old, &id);
    old.supersede_human(
        "bob",
        &action,
        crate::contract::HumanActionStatus::Done,
        Some("saved answer".into()),
    );
    let sent = TranscriptEvent::Delivery {
        id: format!("human-answer:{action}"),
        ts: 1,
        from: Some(DeliveryFrom::new(
            &ThreadId::side(&thread),
            Some(action.clone()),
        )),
        cause: DeliveryCause::Answer {
            action_id: action.clone(),
            status: crate::contract::HumanActionStatus::Done,
            about: "Approve the deployment".into(),
        },
        text: "saved answer".into(),
        receipt: Some(crate::contract::Receipt::Sent),
    };
    old.log
        .append(
            &StreamId::Side(thread.clone()),
            &serde_json::to_value(sent).unwrap(),
        )
        .unwrap();
    let (room, agents) = restart_human_room(old);
    room.recover_exchanges().await;
    done(&room, &id).await;
    until("the agents have two prompts", || {
        agents.prompts().len() == 2
    })
    .await;
    assert!(agents.prompts()[0].contains("saved answer"));
    let delivered: Vec<_> = room
        .log
        .load(&StreamId::Side(thread))
        .into_iter()
        .filter(|event| event["id"] == format!("human-answer:{action}"))
        .collect();
    assert_eq!(
        delivered.iter().next_back().map(|e| e["receipt"].clone()),
        Some(json!("read")),
        "it is read once its turn has begun"
    );
    assert_eq!(delivered.len(), 1, "and it is on the thread once");
}

#[tokio::test]
async fn stopped_or_revoked_human_handoffs_cannot_resume_after_restart() {
    for revoke in [false, true] {
        let (old, id, action) = suspended_handoff("human-handoff-cancel").await;
        let (room, agents) = restart_human_room(old);
        if revoke {
            room.invalidate("bob").unwrap();
        } else {
            room.stop_exchange("ada", "bob").unwrap();
        }
        room.answer_human("bob", &action, crate::contract::HumanAnswer::Done, None)
            .unwrap();
        room.recover_exchanges().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(agents.prompts(), [] as [String; 0]);
        assert_eq!(
            room.exchange_pair("ada~bob").unwrap().requests[0].phase(),
            Phase::Stopped
        );
        assert!(
            room.tape("ada")
                .iter()
                .any(|v| v["id"] == format!("exchange-ended:{id}"))
        );
    }
}

#[tokio::test]
async fn restart_does_not_replay_an_interrupted_human_answer_turn() {
    let (old, id, action) = suspended_handoff("human-answer-interrupted").await;
    let log = old.log.clone();
    drop(old);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let agents = Fake::new(
        Scripted::new(vec![Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        }])
        .gated(gate),
    );
    let active = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    active
        .answer_human("bob", &action, crate::contract::HumanAnswer::Done, None)
        .unwrap();
    until("the agents have one prompt", || agents.prompts().len() == 1).await;
    // A crash snapshot while the answer turn has started but produced no
    // update (not even a read receipt). No concurrent writers share a log.
    let snapshot = scratch("human-answer-crash-snapshot");
    for stream in [
        StreamId::Room,
        StreamId::Tape("ada".into()),
        StreamId::Tape("bob".into()),
        StreamId::Pair("ada~bob".into()),
        StreamId::Inbox("ada".into()),
        StreamId::Inbox("bob".into()),
    ] {
        for event in active.log.load(&stream) {
            snapshot.append(&stream, &event).unwrap();
        }
    }
    active.stop_exchange("ada", "bob").unwrap();
    let agents = Fake::new(Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".into(),
        usage: None,
    }]));
    let room = Room::with_agents(snapshot, Arc::new(DeskKeys), agents.clone());
    room.recover_exchanges().await;
    done(&room, &id).await;
    until("the agents have one prompt", || agents.prompts().len() == 1).await;
    let answered = result(&room, &id).unwrap();
    assert_eq!(answered["cause"]["status"], "failed");
    assert!(
        answered["text"]
            .as_str()
            .unwrap()
            .contains("inspect before retrying")
    );
    assert!(
        agents.told()[0].contains("\"Re: work handoff\" · from"),
        "only the caller hears of it; the recipient's answer turn is not replayed: {:?}",
        agents.told()
    );
}

#[test]
fn a_request_saved_before_work_threads_still_loads_and_has_none() {
    let old = json!({
        "id": "r1", "from": "ada", "to": "bob", "message": "m",
        "intent": "handoff", "phase": "running", "reply": "", "failed": false,
        "started": true, "human_actions": []
    });
    let request =
        Request::from(serde_json::from_value::<Record>(old).expect("an old request loads"));
    assert_eq!(request.thread, None);
    assert_eq!(request.reply_thread, None);
}

/// Agents whose work threads are held at a gate while every other
/// conversation answers at once.
struct HeldWork {
    gate: Arc<tokio::sync::Semaphore>,
}
#[async_trait::async_trait]
impl Agents for HeldWork {
    fn agent(
        &self,
        _persona: &Persona,
        preamble: String,
        _said: Vec<crate::driver::rig::Said>,
        _tools: TeammateTools,
        _mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        let work = preamble.contains("This is a work thread");
        let (id, text) = if work {
            ("work", "handoff finished")
        } else {
            ("dm", "dm answer")
        };
        let script = Scripted::new(vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: id.into(),
                text: text.into(),
            },
            Update::Turn {
                stop_reason: "end_turn".into(),
                usage: None,
            },
        ]);
        Ok(Arc::new(if work {
            script.gated(self.gate.clone())
        } else {
            script
        }))
    }
    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

#[tokio::test]
async fn a_target_answers_the_person_while_its_handoff_is_still_working() {
    let log = scratch("handoff-beside-dm");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(HeldWork { gate: gate.clone() }),
    );
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    until("bob has a working side", || {
        room.sides("bob").iter().any(|side| side.working)
    })
    .await;

    // Bob's own conversation is open while the handoff holds its thread.
    room.start("bob").await.unwrap();
    room.prompt("bob", "how is the weather?", None, None)
        .await
        .unwrap();
    until("bob's tape has the dm answer", || {
        room.tape("bob")
            .iter()
            .any(|v| v["kind"] == "agent" && v["text"] == "dm answer")
    })
    .await;
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase(),
        Phase::Running,
        "the handoff is still working"
    );
    assert!(result(&room, &id).is_none(), "no result yet");

    // It finishes, and the result goes to Ada's inbox.
    gate.add_permits(10);
    done(&room, &id).await;
    assert_eq!(result(&room, &id).unwrap()["text"], "handoff finished");
    assert!(
        room.tape("bob")
            .iter()
            .all(|v| v["text"] != "handoff finished"),
        "the handoff's words never entered Bob's DM"
    );
}

#[tokio::test]
async fn a_handoff_to_a_teammate_with_every_thread_mid_turn_waits_its_place() {
    let log = scratch("handoff-full");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(HeldWork { gate: gate.clone() }),
    );
    room.allow_sender("bob", "ada").unwrap();
    for title in ["One", "Two", "Three"] {
        room.start_side("bob", title).await.unwrap();
    }
    assert_eq!(room.sides("bob").len(), crate::session::sides::MAX_LIVE);
    let id = send(&room, "handoff").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let request = room.exchange_pair("ada~bob").unwrap().requests.remove(0);
    assert_eq!(
        request.phase(),
        Phase::Queued,
        "nothing is interrupted or refused"
    );
    assert_eq!(request.thread, None);
    assert_eq!(room.sides("bob").len(), 3, "none was parked for it");

    gate.add_permits(100);
    done(&room, &id).await;
    assert!(work_thread(&room, &id).len() > 10);
}

#[tokio::test]
async fn a_saved_desk_handoff_cannot_recreate_a_live_calls_grant() {
    let (room, _agents) = setup("lead-call-recovery");
    let mut request = saved_request("desk-request", Intent::Handoff, Phase::Queued);
    request.lead_call = Some(ThreadId::call("ended-call"));
    seed(&room, request, 0, false);
    room.wake_exchange("ada~bob");
    done(&room, "desk-request").await;
    let request = room.exchange_pair("ada~bob").unwrap().requests.remove(0);
    assert!(request.failed());
    assert!(request.reply.contains("lead grant has ended"));
    assert!(request.thread.is_none(), "no work thread was created");
}

#[tokio::test]
async fn a_handoff_at_work_holds_up_nothing_else_between_the_two() {
    let (room, agents) = controlled("handoff-holds-up-nothing");
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("ada", "bob").unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob is at work on it", || {
        agents.drivers["bob"].prompts() == 1
    })
    .await;
    let asked = TeammateTools::new(&room, "bob")
        .call("send", &json!({"to":["ada"],"title":"Which branch?"}))
        .await
        .unwrap();
    let asked = serde_json::from_str::<serde_json::Value>(&asked).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    TeammateTools::new(&room, "ada")
        .call(
            "update_inbox",
            &json!({"items":[{"id": asked, "answer": "main"}]}),
        )
        .await
        .unwrap();
    assert!(
        crate::store::inbox::of(&room.log, "bob")
            .iter()
            .any(|reply| reply.reply_to.as_deref() == Some(asked.as_str())),
        "the answer came back"
    );
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(
        pair.requests
            .iter()
            .find(|r| r.id == handoff)
            .unwrap()
            .phase(),
        Phase::Running,
        "the handoff was still at work when the question was answered"
    );
    agents.drivers["bob"].updates.add_permits(10);
    agents.drivers["ada"].updates.add_permits(10);
    done(&room, &handoff).await;
}
