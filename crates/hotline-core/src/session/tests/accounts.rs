use super::*;
use crate::contract::AgentAccount;

const A: &str = "11111111-1111-4111-8111-111111111111";
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

fn accounts(log: &Log, a_login_at: i64) {
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
    ada.agent_account_id = Some(account_id.to_string());
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
