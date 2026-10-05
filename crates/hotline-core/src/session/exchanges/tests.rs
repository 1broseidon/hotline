mod pause;

use super::*;
use crate::driver::{MessageKind, Update};
use crate::mcp::server::TeammateTools;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use std::time::Duration;

fn setup(name: &str) -> (Arc<Room>, Arc<Fake>) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
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
    let agents = Fake::new(Scripted::turns(turns));
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}
async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("exchange never reached the expected state");
}
async fn send(room: &Arc<Room>, intent: &str) -> String {
    let result = TeammateTools::new(room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":format!("work {intent}"),"intent":intent}),
        )
        .await
        .unwrap();
    serde_json::from_str::<serde_json::Value>(&result).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .into()
}
async fn done(room: &Room, id: &str) {
    until(|| {
        room.exchange_pair("ada~bob").is_some_and(|p| {
            p.requests
                .iter()
                .any(|r| r.id == id && r.phase == Phase::Done)
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
        phase,
        reply: String::new(),
        failed: false,
        reply_counted: false,
        request_counted: false,
        inline: false,
        started: false,
        result_consumed: false,
        human_actions: vec![],
        thread: None,
        reply_thread: None,
    }
}
fn seed(room: &Room, request: Request, count: i64, paused: bool) {
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: count,
        paused,
        requests: vec![request],
    })
    .unwrap();
}

#[tokio::test]
async fn a_handoff_runs_in_its_own_thread_and_returns_the_matching_request_to_the_senders_dm() {
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
    // The result returns to the sender's DM, from the work thread.
    let answer = room
        .tape("ada")
        .into_iter()
        .find(|v| v["cause"]["requestId"] == id)
        .unwrap();
    assert_eq!(answer["text"], "result-0");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["cause"]["threadKey"], "ada~bob");
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
    assert_eq!(room.log.load(&StreamId::Pair("ada~bob".into())).len(), 2);
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
        let answer = room
            .tape("ada")
            .into_iter()
            .find(|v| v["cause"]["requestId"] == id)
            .unwrap();
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
async fn default_ask_never_enters_or_reads_the_recipient_main_conversation() {
    let (room, agents) = setup("ask-isolated");
    room.write_value(
        "bob",
        &json!({"kind":"user","id":"private","ts":1,"text":"PRIVATE MAIN CONTEXT"}),
    );
    let response = TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"bounded review"}),
        )
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    done(&room, response["requestId"].as_str().unwrap()).await;
    assert_eq!(response["intent"], "ask");
    assert!(room.tape("bob").iter().all(|v| v["kind"] != "delivery"));
    assert!(
        lock(&agents.seeds)[0]
            .iter()
            .all(|s| !format!("{s:?}").contains("PRIVATE MAIN CONTEXT"))
    );
    assert!(lock(&agents.preambles)[0].contains("replying privately"));
}

#[tokio::test]
async fn twelve_messages_include_replies_and_switching_intent_does_not_reset_the_brake() {
    let (room, _) = setup("cross-intent-brake");
    room.allow_sender("bob", "ada").unwrap();
    for i in 0..6 {
        let id = send(&room, if i % 2 == 0 { "ask" } else { "handoff" }).await;
        done(&room, &id).await;
    }
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.exchanges, 12);
    assert!(pair.paused);
    let queued = send(&room, "handoff").await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        room.exchange_pair("ada~bob")
            .unwrap()
            .requests
            .last()
            .unwrap()
            .phase,
        Phase::Queued
    );
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, &queued).await;
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 2);
    room.heard_from_person("bob");
    assert_eq!(room.exchange_pair("ada~bob").unwrap().exchanges, 0);
}

#[tokio::test]
async fn the_twelfth_request_keeps_its_reply_queued_until_keep_going() {
    let (room, _) = setup("brake-reply");
    seed(
        &room,
        saved_request("twelfth", Intent::Ask, Phase::Queued),
        11,
        false,
    );
    room.recover_queued_exchanges();
    until(|| room.exchange_pair("ada~bob").unwrap().requests[0].phase == Phase::Reply).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert!(pair.paused);
    assert_eq!(pair.exchanges, 12);
    assert!(!pair.requests[0].reply.is_empty());
    assert!(room.tape("ada").iter().all(|v| v["kind"] != "delivery"));
    room.resume_exchange("ada", "bob").unwrap();
    done(&room, "twelfth").await;
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
    assert!(agents.prompts().is_empty());
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
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
    request.started = true;
    request.request_counted = true;
    seed(&room, request, 1, false);
    room.reconcile_exchanges();
    room.recover_queued_exchanges();
    done(&room, "started").await;
    assert_eq!(agents.prompts().len(), 1, "only sender hears the failure");
    assert!(room.tape("bob").is_empty());
    let result = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "delivery")
        .unwrap();
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
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
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
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Stopped
    );
    assert!(agents.prompts().is_empty());
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
        requests,
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
async fn a_legacy_grant_requires_informed_handoff_approval_but_still_allows_ask() {
    let (room, _) = setup("legacy-handoff-grant");
    let mut target = room.persona("bob").unwrap();
    target.allowed_senders.push("ada".into());
    crate::room::append_persona(&room.log, &target).unwrap();
    let ask = send(&room, "ask").await;
    done(&room, &ask).await;
    let handoff = send(&room, "handoff").await;
    until(|| {
        room.tape("ada")
            .iter()
            .any(|v| v["kind"] == "permission" && v.get("decision").is_none())
    })
    .await;
    let card = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "permission" && v.get("decision").is_none())
        .unwrap();
    assert!(
        card["title"]
            .as_str()
            .unwrap()
            .contains("handoffs in a work thread of its own")
    );
    assert!(room.tape("bob").iter().all(|v| v["kind"] != "delivery"));
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow_always")
        .await
        .unwrap();
    done(&room, &handoff).await;
    let next = send(&room, "handoff").await;
    done(&room, &next).await;
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
    until(|| agents.prompts().len() == 1).await;
    let id = send(&room, "handoff").await;
    // It does not wait for the person's turn: it has a thread of its own.
    until(|| agents.prompts().len() == 2).await;
    assert!(room.mid_turn("bob"), "the person's turn is still going");
    let thread = work_thread(&room, &id);
    assert_eq!(room.sides("bob").len(), 1);
    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Stopped
    );
    assert!(room.sides("bob").is_empty(), "{thread} closed");
    assert!(
        room.mid_turn("bob"),
        "stopping the handoff is not stopping the person"
    );
    gate.add_permits(10);
    until(|| !room.mid_turn("bob")).await;
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
        let tape = room.tape("ada");
        let results: Vec<_> = tape
            .iter()
            .filter(|v| v["cause"]["requestId"] == id)
            .collect();
        assert_eq!(results.len(), 1);
        let pair = room.exchange_pair("ada~bob").unwrap();
        assert_eq!(
            results[0]["text"],
            pair.requests.iter().find(|r| r.id == id).unwrap().reply
        );
    }
}

#[tokio::test]
async fn recovery_and_the_result_worker_dispatch_a_saved_reply_only_once() {
    let (room, agents) = setup("reply-recovery-dedup");
    let mut request = saved_request("result-restart", Intent::Ask, Phase::Reply);
    request.reply = "saved answer".into();
    request.reply_counted = true;
    request.request_counted = true;
    seed(&room, request, 2, false);
    room.write_value(
        "ada",
        &json!({"kind":"delivery", "id":"exchange-result:result-restart",
        "ts": now_ms(), "receipt":"sent", "text":"saved answer", "cause": {
            "kind":"peer", "requestId":"result-restart", "personaId":"bob", "name":"Bob",
            "threadKey":"ada~bob", "status":"done", "about":"saved work"
        }}),
    );
    room.recover_exchanges().await;
    done(&room, "result-restart").await;
    until(|| !agents.prompts().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts().len(), 1);
    assert_eq!(
        room.tape("ada")
            .iter()
            .filter(|v| v["kind"] == "delivery")
            .count(),
        1
    );
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
    cancels: AtomicUsize,
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
            cancels: AtomicUsize::new(0),
        })
    }
    fn prompts(&self) -> usize {
        self.prompts.load(Ordering::SeqCst)
    }
    fn cancels(&self) -> usize {
        self.cancels.load(Ordering::SeqCst)
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
        self.cancels.fetch_add(1, Ordering::SeqCst);
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
    until(|| agents.drivers["bob"].prompts() == 1).await;
    // Cross the real startup recovery deadline, not a substitute recovery path.
    tokio::time::sleep(Duration::from_millis(5300)).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.requests[0].phase, Phase::Running);
    assert!(pair.requests[0].started);
    agents.drivers["bob"].updates.add_permits(2);
    done(&room, &id).await;
    let pair = room.exchange_pair("ada~bob").unwrap();
    assert_eq!(pair.requests[0].reply, "true result");
    assert!(!pair.requests[0].failed);
    agents.drivers["ada"].updates.add_permits(2);
}

async fn nested_handoff(active: bool, expire_dependency_only: bool) {
    let (room, agents) = controlled(if active {
        if expire_dependency_only {
            "nested-scope-active"
        } else {
            "nested-revoke-active"
        }
    } else {
        "nested-revoke-queued"
    });
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("cara", "bob").unwrap();
    if !active {
        room.start("cara").await.unwrap();
        room.prompt("cara", "person's task", None, None)
            .await
            .unwrap();
        until(|| agents.drivers["cara"].prompts() == 1).await;
    }
    let _outer = send(&room, "ask").await;
    until(|| agents.drivers["bob"].prompts() == 1).await;
    // These are the actual delegated tools issued to B's side session for A.
    let tools = lock(&agents.tools)["bob"].clone();
    let nested = tokio::spawn(async move {
        tools
            .call(
                "message_teammate",
                &json!({"to":"cara", "message":"nested task", "intent":"handoff"}),
            )
            .await
    });
    until(|| {
        room.exchange_pair("bob~cara")
            .is_some_and(|p| p.requests[0].phase == Phase::Running)
    })
    .await;
    if active {
        until(|| agents.drivers["cara"].prompts() == 1).await;
    }
    if expire_dependency_only {
        // Stopping A-B revokes the scoped delegated authority without changing
        // A's room epoch. B-C's running worker must notice that dependency too.
        room.stop_exchange("ada", "bob").unwrap();
    } else {
        room.invalidate("ada").unwrap();
    }
    until(|| room.exchange_pair("bob~cara").unwrap().requests[0].phase == Phase::Stopped).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), nested)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    if active {
        assert!(agents.drivers["cara"].cancels() > 0);
        until(|| !room.mid_turn("cara")).await;
    } else {
        // The handoff had its own thread beside the person's turn, and
        // stopping it closed that thread (this fake shares one driver between
        // threads, so it cannot tell whose turn a cancel reached).
        assert!(room.sides("cara").is_empty());
        agents.drivers["cara"].updates.add_permits(2);
        until(|| !room.mid_turn("cara")).await;
    }
}
#[tokio::test]
async fn three_party_revocation_refuses_a_handoff_waiting_behind_the_person() {
    nested_handoff(false, false).await;
}
#[tokio::test]
async fn three_party_revocation_cancels_only_the_active_handoff() {
    nested_handoff(true, false).await;
}
#[tokio::test]
async fn a_running_handoff_continuously_checks_its_scoped_dependency() {
    nested_handoff(true, true).await;
}

#[tokio::test]
async fn stop_at_twelfth_reply_invalidates_dispatched_result_without_cancelling_persons_turn() {
    let (room, agents) = controlled("stop-dispatched-result");
    room.allow_sender("bob", "ada").unwrap();
    room.start("ada").await.unwrap();
    room.prompt("ada", "person still working", None, None)
        .await
        .unwrap();
    until(|| agents.drivers["ada"].prompts() == 1).await;
    let mut reply = saved_request("twelfth-reply", Intent::Ask, Phase::Reply);
    reply.request_counted = true;
    reply.reply = "queued answer".into();
    seed(&room, reply, 11, false);
    room.recover_queued_exchanges();
    done(&room, "twelfth-reply").await;
    assert!(room.exchange_pair("ada~bob").unwrap().paused);
    assert!(!room.exchange_pair("ada~bob").unwrap().requests[0].result_consumed);
    room.stop_exchange("ada", "bob").unwrap();
    assert!(room.mid_turn("ada"));
    assert_eq!(agents.drivers["ada"].cancels(), 0);
    agents.drivers["ada"].updates.add_permits(2);
    until(|| !room.mid_turn("ada")).await;
    assert_eq!(agents.drivers["ada"].prompts(), 1);
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    assert_eq!(
        crate::session::tests::words(lock(&agents.seeds).last().unwrap().clone()),
        [
            crate::driver::rig::Said::User("person still working".into()),
            crate::driver::rig::Said::Agent("true result".into()),
        ],
        "a stopped queued result must not reappear in the restarted main history"
    );
}

#[tokio::test]
async fn stop_settles_pending_approval_and_the_next_request_does_not_wait_for_its_deadline() {
    let (room, _) = setup("stop-pending-approval");
    // Legacy grant triggers informed approval only for Handoff.
    let mut bob = room.persona("bob").unwrap();
    bob.allowed_senders.push("ada".into());
    crate::room::append_persona(&room.log, &bob).unwrap();
    let old = send(&room, "handoff").await;
    until(|| {
        room.tape("ada")
            .iter()
            .any(|e| e["kind"] == "permission" && e.get("decision").is_none())
    })
    .await;
    room.stop_exchange("ada", "bob").unwrap();
    let card = room
        .tape("ada")
        .into_iter()
        .find(|e| e["kind"] == "permission")
        .unwrap();
    assert_eq!(card["decision"], "expired");
    assert!(
        room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow_always")
            .await
            .is_err()
    );
    let next = send(&room, "ask").await;
    tokio::time::timeout(Duration::from_secs(2), done(&room, &next))
        .await
        .unwrap();
    assert_eq!(
        room.exchange_pair("ada~bob")
            .unwrap()
            .requests
            .iter()
            .find(|r| r.id == old)
            .unwrap()
            .phase,
        Phase::Stopped
    );
}

#[tokio::test]
async fn stop_during_uncached_peer_startup_never_executes_ask_and_releases_the_worker() {
    let (room, agents) = controlled("stop-slow-peer-startup");
    agents.drivers["bob"].startup.forget_permits(100);
    let old = send(&room, "ask").await;
    until(|| agents.drivers["bob"].starts.load(Ordering::SeqCst) == 1).await;
    room.stop_exchange("ada", "bob").unwrap();
    until(|| agents.drivers["bob"].cancels() > 0).await;
    assert_eq!(agents.drivers["bob"].prompts(), 0);
    assert_eq!(
        room.exchange_pair("ada~bob")
            .unwrap()
            .requests
            .iter()
            .find(|r| r.id == old)
            .unwrap()
            .phase,
        Phase::Stopped
    );
    until(|| lock(&room.exchange_workers).is_empty()).await;
    agents.drivers["bob"].startup.add_permits(1);
    agents.drivers["bob"].updates.add_permits(10);
    agents.drivers["ada"].updates.add_permits(10);
    let next = send(&room, "ask").await;
    tokio::time::timeout(Duration::from_secs(2), done(&room, &next))
        .await
        .unwrap();
    assert_eq!(agents.drivers["bob"].prompts(), 1);
}

#[tokio::test]
async fn turn_admission_refuses_expired_dependency_before_the_worker_observes_it() {
    let (room, _) = setup("handoff-admission-lease");
    let origin = room.capability_lease("ada");
    let delegated = room.capability_lease("bob").with_dependency(&origin);
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
    assert!(room.begin_handoff("expired-at-admission").is_err());
    assert!(!room.exchange_pair("ada~bob").unwrap().requests[0].started);
}

#[tokio::test]
async fn revocation_invalidates_a_dispatched_result_waiting_behind_the_person() {
    let (room, agents) = controlled("revoke-dispatched-result");
    room.start("ada").await.unwrap();
    room.prompt("ada", "person still working", None, None)
        .await
        .unwrap();
    until(|| agents.drivers["ada"].prompts() == 1).await;
    let mut reply = saved_request("revoked-result", Intent::Ask, Phase::Reply);
    reply.reply = "queued answer".into();
    seed(&room, reply, 1, false);
    room.recover_queued_exchanges();
    done(&room, "revoked-result").await;
    room.invalidate("bob").unwrap();
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Stopped
    );
    assert_eq!(agents.drivers["ada"].cancels(), 0);
    agents.drivers["ada"].updates.add_permits(2);
    until(|| !room.mid_turn("ada")).await;
    assert_eq!(agents.drivers["ada"].prompts(), 1);
}

/// Drive the real handoff and request_human handlers, stopping only the fake
/// model's updates so the tool call happens inside its actual handoff turn.
async fn suspended_handoff(name: &str) -> (Arc<Room>, String, String) {
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
    until(|| agents.prompts().len() == 1).await;
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
    gate.add_permits(2);
    until(|| {
        room.exchange_pair("ada~bob").unwrap().requests[0].phase == Phase::WaitingHuman
            && !room.mid_turn("bob")
    })
    .await;
    assert!(
        room.tape("ada")
            .iter()
            .all(|v| v["cause"]["requestId"] != id),
        "waiting is not a final result"
    );
    (room, id, action)
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
        until(|| agents.prompts().len() == 2).await;
        let result = room
            .tape("ada")
            .into_iter()
            .find(|v| v["cause"]["requestId"] == id)
            .unwrap();
        assert_eq!(result["text"], "Finished after the human answer");
        assert_eq!(result["cause"]["status"], "done");
        assert!(agents.prompts()[0].contains("operator note"));
        assert!(!agents.prompts()[0].contains("work handoff"));
        room.recover_exchanges().await;
        assert_eq!(
            room.tape("ada")
                .iter()
                .filter(|v| v["cause"]["requestId"] == id)
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
            until(|| room.exchange_pair("ada~bob").unwrap().requests[0].phase == Phase::Reply)
                .await;
            assert_eq!(
                agents.prompts().len(),
                1,
                "the existing work continues, but its automatic reply waits at the cap"
            );
            room.resume_exchange("ada", "bob").unwrap();
        }
        done(&room, &id).await;
        until(|| agents.prompts().len() == 2).await;
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
    until(|| agents.prompts().len() == 2).await;
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

/// A colleague's result for a work thread is admitted when its turn begins,
/// as the main conversation's is: one the exchange was stopped on while the
/// thread was busy is never heard.
#[tokio::test]
async fn a_result_queued_for_a_busy_work_thread_is_stopped_with_its_exchange() {
    let (room, agents) = controlled("revoke-work-result");
    let side = room
        .start_side("ada", "the person's task")
        .await
        .unwrap()
        .side_id;
    until(|| agents.drivers["ada"].prompts() == 1).await;
    let mut reply = saved_request("stopped-work-result", Intent::Ask, Phase::Reply);
    reply.reply = "queued answer".into();
    reply.reply_thread = Some(side.clone());
    seed(&room, reply, 1, false);
    room.recover_queued_exchanges();
    done(&room, "stopped-work-result").await;
    room.invalidate("bob").unwrap();
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Stopped,
        "a result that has not begun a turn can still be stopped"
    );
    agents.drivers["ada"].updates.add_permits(4);
    until(|| !room.mid_turn("ada")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        agents.drivers["ada"].prompts(),
        1,
        "the queued result never ran"
    );
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
        assert!(agents.prompts().is_empty());
        assert_eq!(
            room.exchange_pair("ada~bob").unwrap().requests[0].phase,
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
async fn a_human_answer_does_not_replace_collaboration_consent_after_restart() {
    let (old, id, action) = suspended_handoff("human-handoff-denied").await;
    // Model a session-only approval: neither a standing sender nor informed
    // standing consent is available to the new process.
    let mut target = old.persona("bob").unwrap();
    target.allowed_senders.clear();
    crate::room::append_persona(&old.log, &target).unwrap();
    old.forget_informed_collaboration("bob");
    let mut caller = old.persona("ada").unwrap();
    caller.reach = Some(crate::contract::Reach::Workspace);
    crate::room::append_persona(&old.log, &caller).unwrap();
    let (room, agents) = restart_human_room(old);
    room.answer_human("bob", &action, crate::contract::HumanAnswer::Done, None)
        .unwrap();
    until(|| {
        room.tape("ada")
            .iter()
            .any(|v| v["kind"] == "permission" && v.get("decision").is_none())
    })
    .await;
    assert!(
        agents.prompts().is_empty(),
        "human approval grants no collaboration authority"
    );
    let card = room
        .tape("ada")
        .into_iter()
        .find(|v| v["kind"] == "permission" && v.get("decision").is_none())
        .unwrap();
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "deny")
        .await
        .unwrap();
    done(&room, &id).await;
    let result = room
        .tape("ada")
        .into_iter()
        .find(|v| v["cause"]["requestId"] == id)
        .unwrap();
    assert_eq!(result["cause"]["status"], "failed");
    assert!(room.tape("bob").iter().all(|v| v["id"] != "finished"));
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
    until(|| agents.prompts().len() == 1).await;
    // A crash snapshot while the answer turn has started but produced no
    // update (not even a read receipt). No concurrent writers share a log.
    let snapshot = scratch("human-answer-crash-snapshot");
    for stream in [
        StreamId::Room,
        StreamId::Tape("ada".into()),
        StreamId::Tape("bob".into()),
        StreamId::Pair("ada~bob".into()),
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
    until(|| agents.prompts().len() == 1).await;
    let result = room
        .tape("ada")
        .into_iter()
        .find(|v| v["cause"]["requestId"] == id)
        .unwrap();
    assert_eq!(result["cause"]["status"], "failed");
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("inspect before retrying")
    );
    assert!(
        agents.prompts()[0].contains("inspect before retrying"),
        "only the caller hears the uncertainty; the recipient's answer turn is not replayed"
    );
}

#[test]
fn a_request_saved_before_work_threads_still_loads_and_has_none() {
    let old = json!({
        "id": "r1", "from": "ada", "to": "bob", "message": "m",
        "intent": "handoff", "phase": "running", "reply": "", "failed": false,
        "started": true, "humanActions": []
    });
    let request: Request = serde_json::from_value(old).expect("an old request loads");
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
    until(|| room.sides("bob").iter().any(|side| side.working)).await;

    // Bob's own conversation is open while the handoff holds its thread.
    room.start("bob").await.unwrap();
    room.prompt("bob", "how is the weather?", None, None)
        .await
        .unwrap();
    until(|| {
        room.tape("bob")
            .iter()
            .any(|v| v["kind"] == "agent" && v["text"] == "dm answer")
    })
    .await;
    assert_eq!(
        room.exchange_pair("ada~bob").unwrap().requests[0].phase,
        Phase::Running,
        "the handoff is still working"
    );
    assert!(
        room.tape("ada")
            .iter()
            .all(|v| v["cause"]["requestId"] != id),
        "no result yet"
    );

    // It finishes, and the result goes to Ada's DM.
    gate.add_permits(10);
    done(&room, &id).await;
    until(|| {
        room.tape("ada")
            .iter()
            .any(|v| v["cause"]["requestId"] == id)
    })
    .await;
    let result = room
        .tape("ada")
        .into_iter()
        .find(|v| v["cause"]["requestId"] == id)
        .unwrap();
    assert_eq!(result["text"], "handoff finished");
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
        request.phase,
        Phase::Queued,
        "nothing is interrupted or refused"
    );
    assert_eq!(request.thread, None);
    assert_eq!(room.sides("bob").len(), 3, "none was parked for it");

    gate.add_permits(100);
    done(&room, &id).await;
    assert!(work_thread(&room, &id).len() > 10);
}
