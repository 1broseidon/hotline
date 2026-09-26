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
async fn handoff_uses_main_context_and_returns_the_matching_request_without_another_tool_call() {
    let (room, agents) = setup("handoff-context-result");
    room.write_value(
        "bob",
        &json!({"kind":"user","id":"secret","ts":1,"text":"recipient main context"}),
    );
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "handoff").await;
    done(&room, &id).await;
    let target = room
        .tape("bob")
        .into_iter()
        .find(|v| v["cause"]["kind"] == "handoff")
        .unwrap();
    assert_eq!(target["cause"]["requestId"], id);
    assert_eq!(target["receipt"], "read");
    let answer = room
        .tape("ada")
        .into_iter()
        .find(|v| v["cause"]["requestId"] == id)
        .unwrap();
    assert_eq!(answer["text"], "result-0");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["cause"]["threadKey"], "ada~bob");
    assert!(
        lock(&agents.seeds)[0]
            .iter()
            .any(|s| format!("{s:?}").contains("recipient main context"))
    );
    assert_eq!(room.log.load(&StreamId::Thread("ada~bob".into())).len(), 2);
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
            .contains("handoffs into its main conversation")
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
async fn handoff_waits_behind_the_persons_turn_and_stop_does_not_cancel_that_turn() {
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
    until(|| {
        room.tape("bob")
            .iter()
            .any(|v| v["cause"]["requestId"] == id)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(agents.prompts().len(), 1);
    assert_eq!(agents.cancel_count(), 0);
    room.stop_exchange("ada", "bob").unwrap();
    assert_eq!(
        agents.cancel_count(),
        0,
        "the person's work is not this handoff"
    );
    gate.add_permits(1);
    until(|| !room.mid_turn("bob")).await;
    assert_eq!(
        agents.prompts().len(),
        1,
        "stopped handoff never reaches the driver"
    );
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
}
#[async_trait::async_trait]
impl Agents for ControlledAgents {
    fn agent(
        &self,
        persona: &Persona,
        _preamble: String,
        _said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        _mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
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
        assert_eq!(agents.drivers["cara"].cancels(), 0);
        assert!(room.mid_turn("cara"));
        agents.drivers["cara"].updates.add_permits(2);
        until(|| !room.mid_turn("cara")).await;
        assert_eq!(agents.drivers["cara"].prompts(), 1);
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
