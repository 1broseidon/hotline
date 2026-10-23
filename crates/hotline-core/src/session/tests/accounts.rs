use super::*;
use crate::contract::{
    AccountHold, AccountStatusState, AccountStatusView, AgentAccount, SessionAccount, ThreadKind,
};

pub(super) const A: &str = "11111111-1111-4111-8111-111111111111";
const B: &str = "22222222-2222-4222-8222-222222222222";

fn account(id: &str, name: &str, login_at: i64) -> AgentAccount {
    AgentAccount {
        id: id.to_string(),
        backend_id: "claude-acp".to_string(),
        name: name.to_string(),
        created_at: 1,
        login_at: Some(login_at),
        setup: None,
    }
}

pub(super) fn accounts(log: &Log, a_login_at: i64) {
    room::write_agent_accounts(
        log,
        &[
            account(A, "Account A", a_login_at),
            account(B, "Account B", 1),
        ],
    )
    .unwrap();
}

fn on_account(log: &Log, account_id: &str) -> Persona {
    let mut ada = persona("ada");
    ada.backend_id = "claude-acp".to_string();
    ada.account_id = Some(account_id.to_string());
    ada.cwd = log.root().join("ada").to_string_lossy().into_owned();
    ada
}

fn checkpoint(account_id: &str, session_id: &str) -> SessionCheckpoint {
    SessionCheckpoint {
        backend_id: "claude-acp".to_string(),
        session_id: session_id.to_string(),
        account_id: Some(account_id.to_string()),
    }
}

fn saved(room: &Room) -> Vec<(String, Option<String>)> {
    room::roster(&room.log)
        .into_iter()
        .find(|persona| persona.id == "ada")
        .map(|persona| {
            persona
                .session_checkpoints
                .into_iter()
                .map(|checkpoint| (checkpoint.session_id, checkpoint.account_id))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_checkpoint_is_kept_only_while_the_record_names_the_sessions_account_and_sign_in() {
    for (record, login_at, kept) in [(A, 1, true), (B, 1, false), (A, 2, false)] {
        let agents = Fake::new(Scripted::turns(vec![
            saying("t1", "one"),
            saying("t2", "two"),
        ]));
        agents.reporting("s1", false);
        let log = scratch(&format!("account-checkpoint-{record}-{login_at}"));
        accounts(&log, 1);
        enrol(&log, &on_account(&log, A));
        let room = Room::with_agents(log.clone(), Arc::new(DeskKeys), agents);
        room.start("ada").await.unwrap();
        enrol(&log, &on_account(&log, record));
        accounts(&log, login_at);
        room.prompt("ada", "first", None, None).await.unwrap();
        room.prompt("ada", "second", None, None).await.unwrap();
        until("the second turn is on the tape", || {
            tape(&room, "ada")
                .iter()
                .any(|event| event["text"] == "two")
        })
        .await;
        let expected = if kept {
            vec![("s1".to_string(), Some(A.to_string()))]
        } else {
            Vec::new()
        };
        assert_eq!(saved(&room), expected, "{record} {login_at}");
    }
}

#[tokio::test]
async fn a_sign_in_that_lands_while_the_session_starts_is_not_the_sessions() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let log = scratch("account-sign-in-in-flight");
    accounts(&log, 1);
    enrol(&log, &on_account(&log, A));
    let signed_in_again = log.clone();
    agents.when_building(move || accounts(&signed_in_again, 2));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("ada").await.unwrap();
    let session = lock(&room.sessions).get("ada").cloned().unwrap();
    assert_eq!(
        (session.account_id.as_deref(), session.account_login_at),
        (Some(A), Some(1))
    );
}

#[tokio::test]
async fn a_resume_onto_another_account_opens_fresh() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let log = scratch("account-resume");
    accounts(&log, 1);
    let mut ada = on_account(&log, B);
    ada.session_checkpoints = vec![checkpoint(A, "s-old"), checkpoint(B, "s-b")];
    enrol(&log, &ada);
    let t = now_ms();
    write_tape(
        &log,
        "ada",
        &[
            json!({
                "kind": "chapter", "id": "c1", "ts": t - 10_000, "backendId": "claude-acp",
                "accountId": A, "sessionId": "s-old", "endedAt": t - 5_000,
                "title": "Crane jam", "closedBy": "idle",
            }),
            spoken("user", "u1", t - 9_000, "did the crane jam?"),
            json!({"kind": "chapter", "id": "c2", "ts": t - 1_000, "backendId": "claude-acp"}),
        ],
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("ada").await.unwrap();
    room.resume_chapter("ada").await.unwrap();
    assert!(saved(&room).is_empty(), "{:?}", saved(&room));
    let marker = markers(&room, "ada").pop().unwrap();
    assert_eq!(
        (marker["accountId"].as_str(), marker.get("sessionId")),
        (Some(B), None),
        "{marker}"
    );
}

#[tokio::test]
async fn an_unknown_account_refuses_the_start_in_public_words() {
    let log = scratch("account-unknown");
    enrol(
        &log,
        &on_account(&log, "33333333-3333-4333-8333-333333333333"),
    );
    let room = Room::new(log, Arc::new(DeskKeys));
    assert_eq!(
        room.start("ada").await.unwrap_err(),
        "Ada can't start: its Claude Code account needs attention at the desk."
    );
}

#[tokio::test]
async fn a_claude_teammate_set_to_an_account_it_ignores_runs_on_none_and_keeps_its_checkpoint() {
    const ELSEWHERE: &str = "33333333-3333-4333-8333-333333333333";
    for kind in ["credential", "elsewhere"] {
        let log = scratch(&format!("account-ignored-{kind}"));
        let vault = Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap();
        let credential = vault.create("anthropic", "Work", "not-a-real-key").unwrap();
        room::write_agent_accounts(
            &log,
            &[AgentAccount {
                backend_id: "codex-acp".to_string(),
                ..account(ELSEWHERE, "Elsewhere", 1)
            }],
        )
        .unwrap();
        let ignored = if kind == "credential" {
            credential.id.as_str()
        } else {
            ELSEWHERE
        };
        let mut ada = on_account(&log, ignored);
        ada.session_checkpoints = vec![SessionCheckpoint {
            backend_id: "claude-acp".to_string(),
            session_id: "s-old".to_string(),
            account_id: None,
        }];
        enrol(&log, &ada);
        let agents = Fake::new(Scripted::new(saying("1", "Done.")));
        agents.reporting("s-new", false);
        let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
        started(&room).await;
        assert_eq!(running(&room), None, "{kind}");
        assert_eq!(saved(&room), [("s-old".to_string(), None)], "{kind}");
        room.prompt("ada", "Work on it.", None, None).await.unwrap();
        until("the new session is kept", || {
            saved(&room) == [("s-new".to_string(), None)]
        })
        .await;
        room.stop("ada").unwrap();
        room.assign_account("ada", None).await.unwrap();
        assert_eq!(saved(&room), [("s-new".to_string(), None)], "{kind}");
    }
}

fn on_accounts(name: &str, agents: Arc<Fake>) -> (Arc<Room>, String, String) {
    let log = scratch(name);
    accounts(&log, 1);
    enrol(&log, &on_account(&log, A));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    (room, A.to_string(), B.to_string())
}

pub(super) fn gated() -> (Arc<Fake>, Arc<Semaphore>) {
    let gate = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::new(saying("1", "Working.")).gated(gate.clone()));
    (agents, gate)
}

pub(super) async fn started(room: &Arc<Room>) {
    room.start("ada").await.unwrap();
    until_state(room, "ada", SessionState::Ready).await;
}

pub(super) async fn mid_turn(room: &Arc<Room>) {
    room.prompt("ada", "Work on it.", None, None).await.unwrap();
    until_state(room, "ada", SessionState::Thinking).await;
}

pub(super) fn running(room: &Room) -> Option<String> {
    room.info("ada").account.and_then(|account| account.running)
}

pub(super) fn chosen(room: &Room) -> Option<String> {
    room.persona("ada").unwrap().account_id
}

pub(super) fn switching(running: &str, wanted: &str, held: Option<AccountHold>) -> SessionAccount {
    SessionAccount {
        running: Some(running.to_string()),
        wanted: Some(wanted.to_string()),
        held,
    }
}

fn status(state: AccountStatusState) -> AccountStatusView {
    AccountStatusView {
        state,
        checked_at: None,
        email: None,
        plan: None,
    }
}

fn signed_in_at(log: &Log, account_id: &str, login_at: i64) {
    let mut rows = room::agent_accounts(log);
    for row in rows.iter_mut().filter(|row| row.id == account_id) {
        row.login_at = Some(login_at);
    }
    room::write_agent_accounts(log, &rows).unwrap();
}

fn both_checkpoints(room: &Room, a: &str, b: &str) {
    room::checkpoint_session(&room.log, "ada", "claude-acp", Some(a), "s-a").unwrap();
    room::checkpoint_session(&room.log, "ada", "claude-acp", Some(b), "s-b-old").unwrap();
}

#[tokio::test]
async fn a_switch_during_a_dm_turn_waits_for_the_turn_and_then_applies() {
    let (agents, gate) = gated();
    let (room, a, b) = on_accounts("switch-dm-turn", agents.clone());
    started(&room).await;
    mid_turn(&room).await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, None)
    );
    assert_eq!(agents.views().len(), 1);
    gate.add_permits(10);
    until("switched", || running(&room) == Some(b.clone())).await;
    assert_eq!(agents.views().last().unwrap().account_id, Some(b));
}

#[tokio::test]
async fn a_switch_waits_for_a_side_thread_mid_turn() {
    let (agents, gate) = gated();
    let (room, a, b) = on_accounts("switch-side", agents.clone());
    started(&room).await;
    room.start_side("ada", "Task").await.unwrap();
    until("the side is working", || {
        room.sides("ada").first().is_some_and(|side| side.working)
    })
    .await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, None)
    );
    room.start_side("ada", "Another task").await.unwrap();
    until("both sides have agents", || agents.views().len() == 3).await;
    assert_eq!(agents.views().last().unwrap().account_id, Some(a));
    gate.add_permits(10);
    until("switched", || running(&room) == Some(b.clone())).await;
}

#[tokio::test]
async fn a_switch_waits_for_a_call_thread_mid_turn() {
    let (agents, gate) = gated();
    let (room, a, b) = on_accounts("switch-call", agents.clone());
    started(&room).await;
    let thread = ThreadId {
        kind: ThreadKind::Call,
        key: "call-1".into(),
    };
    let (_live, launch) = room
        .bring_up(
            "ada",
            super::sides::Start {
                thread: thread.clone(),
                title: "Call".into(),
                started: now_ms(),
                opener: None,
                fresh: true,
                saved: None,
                handoff: None,
            },
        )
        .unwrap();
    room.prompt_side(&thread.key, "Run the tests", None)
        .await
        .unwrap();
    room.launch(launch);
    until("the call has an agent", || agents.views().len() == 2).await;
    assert_eq!(agents.views()[1].account_id, Some(a.clone()));
    assert!(!room.working_calls("ada").is_empty());
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, None)
    );
    assert_eq!(running(&room), Some(a));
    gate.add_permits(10);
    until("switched", || running(&room) == Some(b.clone())).await;
}

#[tokio::test]
async fn keeping_the_running_account_clears_the_switch_and_keeps_checkpoints() {
    let (agents, gate) = gated();
    let (room, a, b) = on_accounts("switch-keep", agents.clone());
    room::checkpoint_session(&room.log, "ada", "claude-acp", Some(&a), "s-a").unwrap();
    agents.reporting("s-a", true);
    started(&room).await;
    mid_turn(&room).await;
    room.assign_account("ada", Some(b)).await.unwrap();
    assert_eq!(
        room.assign_account("ada", Some(a.clone())).await.unwrap(),
        switching(&a, &a, None)
    );
    gate.add_permits(10);
    until_state(&room, "ada", SessionState::Ready).await;
    assert_eq!(agents.views().len(), 1);
    assert_eq!(saved(&room), [("s-a".to_string(), Some(a))]);
}

#[tokio::test]
async fn a_held_account_waits_for_its_sign_in() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let (room, a, b) = on_accounts("switch-held", agents.clone());
    started(&room).await;
    room.account_statuses()
        .put(&b, status(AccountStatusState::SignedOut));
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, Some(AccountHold::NotSignedIn))
    );
    assert_eq!(chosen(&room), Some(b.clone()));
    assert_eq!(agents.views().len(), 1);
    signed_in_at(&room.log, &b, 2);
    room.account_statuses()
        .put(&b, status(AccountStatusState::SignedIn));
    room.account_signed_in(&b);
    until("switched", || running(&room) == Some(b.clone())).await;
}

#[tokio::test]
async fn applying_clears_the_backends_checkpoints_and_the_next_start_is_new() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let (room, a, b) = on_accounts("switch-apply", agents.clone());
    both_checkpoints(&room, &a, &b);
    started(&room).await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&b, &b, None)
    );
    assert_eq!(saved(&room), []);
    let view = agents.views().pop().unwrap();
    assert_eq!(
        (view.account_id, view.session_checkpoints),
        (Some(b), Vec::new())
    );
}

#[tokio::test]
async fn stop_and_switch_now_starts_the_new_account_fresh() {
    let (agents, _gate) = gated();
    let (room, a, b) = on_accounts("switch-stop", agents.clone());
    both_checkpoints(&room, &a, &b);
    started(&room).await;
    mid_turn(&room).await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, None)
    );
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    until("the new session is on B, fresh", || {
        agents.views().last().is_some_and(|view| {
            view.account_id == Some(b.clone()) && view.session_checkpoints.is_empty()
        })
    })
    .await;
    assert_eq!(saved(&room), []);
}

#[tokio::test]
async fn a_stale_lease_cannot_write_a_checkpoint_after_the_switch() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    agents.reporting("s-a", false);
    let (room, a, b) = on_accounts("switch-stale-lease", agents);
    started(&room).await;
    let old = room.session("ada").unwrap();
    assert_eq!(lock(&old.pending_checkpoint).as_deref(), Some("s-a"));
    room.assign_account("ada", Some(b)).await.unwrap();
    room.assign_account("ada", Some(a.clone())).await.unwrap();
    assert_eq!(running(&room), Some(a));
    room.checkpoint(&old);
    assert_eq!(saved(&room), []);
}

#[tokio::test]
async fn signing_in_again_rebuilds_sessions_that_started_before_it() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let (room, a, _b) = on_accounts("switch-relogin", agents.clone());
    started(&room).await;
    signed_in_at(&room.log, &a, 2);
    room::checkpoint_session(&room.log, "ada", "claude-acp", Some(&a), "s-a-late").unwrap();
    room.account_signed_in(&a);
    until("rebuilt on the new sign-in", || {
        room.session("ada")
            .is_ok_and(|session| session.account_login_at == Some(2))
    })
    .await;
    let views = agents.views();
    assert_eq!(views.len(), 2);
    assert_eq!(views[1].session_checkpoints, []);
    assert_eq!(running(&room), Some(a));
    assert_eq!(saved(&room), []);
}

#[tokio::test]
async fn an_assignment_is_checked_before_anything_is_written() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let (room, a, _b) = on_accounts("switch-checked", agents);
    enrol(
        &room.log,
        &Persona {
            name: "Hal".to_string(),
            backend_id: "codex-acp".to_string(),
            ..persona("hal")
        },
    );
    let before = room::roster(&room.log);
    for (persona_id, account_id, refusal) in [
        ("ghost", A, "There is no such teammate in this room."),
        (
            "ada",
            "33333333-3333-4333-8333-333333333333",
            "That account is not on this desk.",
        ),
        ("hal", A, "That agent does not use accounts."),
    ] {
        assert_eq!(
            room.assign_account(persona_id, Some(account_id.to_string()))
                .await,
            Err(refusal.to_string())
        );
    }
    assert_eq!(room::roster(&room.log), before);
    assert_eq!(chosen(&room), Some(a));
}

#[tokio::test]
async fn a_pending_switch_is_dropped_when_its_teammate_is_removed_or_changes_agent() {
    let (agents, _gate) = gated();
    let (room, a, b) = on_accounts("switch-deleted", agents.clone());
    started(&room).await;
    mid_turn(&room).await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, None)
    );
    room.invalidate("ada").unwrap();
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "persona", "id": "ada", "deleted": true}),
        )
        .unwrap();
    room.forget("ada");
    assert_eq!(room.info("ada").account, None);

    let (agents, gate) = gated();
    let (room, a, b) = on_accounts("switch-moved", agents.clone());
    started(&room).await;
    mid_turn(&room).await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, None)
    );
    moved_to(&room, "hotline").await;
    gate.add_permits(10);
    assert_eq!(room.info("ada").account, None);
    assert_eq!(agents.views().last().unwrap().backend_id, "hotline");

    let agents = Fake::new(Scripted::new(Vec::new()));
    let (room, a, b) = on_accounts("switch-held-moved", agents.clone());
    started(&room).await;
    room.account_statuses()
        .put(&b, status(AccountStatusState::SignedOut));
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        switching(&a, &b, Some(AccountHold::NotSignedIn))
    );
    moved_to(&room, "hotline").await;
    assert_eq!(room.info("ada").account, None);
    assert_eq!(agents.views().last().unwrap().account_id, Some(b));

    let (agents, gate) = gated();
    let log = scratch("switch-moved-from-hotline");
    let vault = Vault::open_with_store(
        log.root(),
        log.clone(),
        Arc::new(crate::credentials::MemoryStore::default()),
    )
    .unwrap();
    let credential = vault.create("anthropic", "C", "not-a-real-key").unwrap();
    enrol(
        &log,
        &Persona {
            model_id: Some("anthropic/claude-x".to_string()),
            session_checkpoints: vec![SessionCheckpoint {
                backend_id: "claude-acp".to_string(),
                session_id: "s-old".to_string(),
                account_id: None,
            }],
            ..persona("ada")
        },
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    started(&room).await;
    mid_turn(&room).await;
    room.assign_account("ada", Some(credential.id))
        .await
        .unwrap();
    moved_to(&room, "claude-acp").await;
    gate.add_permits(10);
    assert_eq!(room.info("ada").account, None);
    assert_eq!(saved(&room), [("s-old".to_string(), None)]);
}

/// As the wire moves a teammate to another agent.
async fn moved_to(room: &Arc<Room>, backend_id: &str) {
    room.invalidate("ada").unwrap();
    let mut moved = room.persona("ada").unwrap();
    moved.backend_id = backend_id.to_string();
    room::append_persona(&room.log, &moved).unwrap();
    room.reattach("ada").await.unwrap();
}

#[tokio::test]
async fn stop_and_switch_now_starts_while_the_switch_holds_the_gate() {
    let (agents, _gate) = gated();
    let (room, _a, b) = on_accounts("switch-stop-gate", agents);
    started(&room).await;
    mid_turn(&room).await;
    room.assign_account("ada", Some(b.clone())).await.unwrap();
    let start_gate = room.start_gate("ada");
    let holding = start_gate.clone().lock_owned().await;
    room.stop("ada").unwrap();
    until("the switch waits at the gate", || {
        Arc::strong_count(&start_gate) == 4
    })
    .await;
    let starting = tokio::spawn({
        let room = room.clone();
        async move { room.start("ada").await }
    });
    until("the start waits behind it", || {
        Arc::strong_count(&start_gate) == 5
    })
    .await;
    drop(holding);
    starting.await.unwrap().unwrap();
    assert_eq!(running(&room), Some(b));
}

#[tokio::test]
async fn a_switch_lost_to_a_restart_starts_fresh_on_the_chosen_account() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let log = scratch("switch-lost");
    accounts(&log, 1);
    let mut ada = on_account(&log, B);
    ada.session_checkpoints = vec![checkpoint(A, "s-a1"), checkpoint(B, "s-b-old")];
    enrol(&log, &ada);
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    started(&room).await;
    let view = agents.views().pop().unwrap();
    assert_eq!(
        (view.account_id, view.session_checkpoints),
        (Some(B.to_string()), Vec::new())
    );
    assert_eq!(saved(&room), []);
}

#[tokio::test]
async fn a_switch_taken_at_a_start_lets_go_of_idle_threads_on_the_old_account() {
    let agents = Fake::new(Scripted::new(saying("1", "Done.")));
    let (room, _a, b) = on_accounts("switch-start-sides", agents);
    room.start_side("ada", "Task").await.unwrap();
    until("the side is idle", || {
        room.sides("ada").first().is_some_and(|side| !side.working)
    })
    .await;
    room.account_statuses()
        .put(&b, status(AccountStatusState::SignedOut));
    room.assign_account("ada", Some(b.clone())).await.unwrap();
    assert_eq!(room.sides("ada").len(), 1);
    room.account_statuses()
        .put(&b, status(AccountStatusState::SignedIn));
    room.start("ada").await.unwrap();
    assert!(room.sides("ada").is_empty());
    assert_eq!(running(&room), Some(b));
}

#[tokio::test]
async fn a_rebuild_owed_to_a_sign_in_survives_a_switch_that_was_kept() {
    let (agents, gate) = gated();
    let (room, a, b) = on_accounts("switch-relogin-kept", agents);
    started(&room).await;
    mid_turn(&room).await;
    signed_in_at(&room.log, &a, 2);
    room.account_signed_in(&a);
    room.assign_account("ada", Some(b)).await.unwrap();
    assert_eq!(
        room.assign_account("ada", Some(a.clone())).await.unwrap(),
        switching(&a, &a, None)
    );
    gate.add_permits(10);
    until("rebuilt on the new sign-in", || {
        room.session("ada")
            .is_ok_and(|session| session.account_login_at == Some(2))
    })
    .await;
    assert_eq!(running(&room), Some(a));
}
