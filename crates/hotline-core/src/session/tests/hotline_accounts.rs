//! A Hotline Agent teammate's account: a credential of its model's provider,
//! held by the driver for the session's life, and changed like a harness's.

use super::accounts::{A, accounts, chosen, gated, mid_turn, running, started, switching};
use super::*;
use crate::contract::SessionAccount;
use hotline_room::provider_auth::ChosenAccount;

#[tokio::test]
async fn an_unknown_hotline_account_refuses_the_start_in_public_words() {
    let log = scratch("hotline-account-unknown");
    enrol(
        &log,
        &Persona {
            account_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
            ..persona("ada")
        },
    );
    let room = Room::new(log, Arc::new(DeskKeys));
    assert_eq!(
        room.start("ada").await.unwrap_err(),
        "Ada can't start: its Hotline Agent account needs attention at the desk."
    );
}

/// The account each of the driver's asks names.
#[derive(Default)]
struct Asked(Mutex<Vec<Option<ChosenAccount>>>);

impl ProviderKeys for Asked {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        DeskKeys.provider_auth()
    }

    fn provider_auth_for(&self, account: Option<&ChosenAccount>) -> HashMap<String, ProviderAuth> {
        lock(&self.0).push(account.cloned());
        DeskKeys.provider_auth()
    }
}

#[tokio::test]
async fn a_hotline_driver_holds_a_credential_of_any_provider_and_no_agent_account() {
    let log = scratch("hotline-account-held");
    accounts(&log, 1);
    let vault = Vault::open_with_store(
        log.root(),
        log.clone(),
        Arc::new(crate::credentials::MemoryStore::default()),
    )
    .unwrap();
    let work = vault.create("anthropic", "Work", "not-a-real-key").unwrap();
    let old = vault.create("openai", "Old", "not-a-real-key").unwrap();
    vault.revoke(&old.id).unwrap();
    let held = |credential: &crate::contract::Credential| ChosenAccount {
        id: credential.id.clone(),
        provider_id: credential.provider_id.clone(),
    };
    for (account_id, expected) in [
        (work.id.clone(), Some(held(&work))),
        (old.id.clone(), Some(held(&old))),
        (A.to_string(), None),
    ] {
        enrol(
            &log,
            &Persona {
                account_id: Some(account_id),
                ..persona("ada")
            },
        );
        let keys = Arc::new(Asked::default());
        let room = Room::new(log.clone(), keys.clone());
        room.start("ada").await.unwrap();
        let asked = lock(&keys.0).clone();
        assert!(!asked.is_empty());
        assert!(asked.iter().all(|asked| *asked == expected), "{asked:?}");
    }
}

/// The desk's own agents build each driver, so the account it holds shows in
/// `held`; the scripted one runs in its place, so a turn can be held open.
struct Built {
    desk: DeskAgents,
    scripted: Arc<Fake>,
}

#[async_trait]
impl Agents for Built {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<Said>,
        tools: TeammateTools,
        extra_mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        let built = self.desk.agent(
            persona,
            preamble.clone(),
            said.clone(),
            tools.clone(),
            extra_mcp.clone(),
        )?;
        // Its info is read with the account it holds.
        built
            .current_info()
            .ok_or("The desk's driver gave no info to read its account from.")?;
        self.scripted
            .agent(persona, preamble, said, tools, extra_mcp)
    }

    async fn complete(&self, model_id: &str, system: &str, prompt: &str) -> Result<String, String> {
        self.scripted.complete(model_id, system, prompt).await
    }
}

/// Ada on Hotline Agent and `anthropic/claude-x`, with the Anthropic keys `a`
/// and `b` on the desk.
struct Desk {
    room: Arc<Room>,
    held: Arc<Asked>,
    vault: Vault,
    a: String,
    b: String,
}

fn desk(name: &str, scripted: Arc<Fake>, set_to_a: bool) -> Desk {
    let log = scratch(name);
    let vault = Vault::open_with_store(
        log.root(),
        log.clone(),
        Arc::new(crate::credentials::MemoryStore::default()),
    )
    .unwrap();
    let a = vault.create("anthropic", "A", "not-a-real-key").unwrap().id;
    let b = vault.create("anthropic", "B", "not-a-real-key").unwrap().id;
    enrol(
        &log,
        &Persona {
            model_id: Some("anthropic/claude-x".to_string()),
            account_id: set_to_a.then(|| a.clone()),
            ..persona("ada")
        },
    );
    let held = Arc::new(Asked::default());
    let desk = DeskAgents {
        keys: held.clone(),
        root: log.root().to_path_buf(),
        log: log.clone(),
        mcp_vault: None,
        acp_start: acp::StartBounds::default(),
    };
    let room = Room::with_agents(log, Arc::new(DeskKeys), Arc::new(Built { desk, scripted }));
    Desk {
        room,
        held,
        vault,
        a,
        b,
    }
}

/// The account each driver built so far holds, in order.
fn held(desk: &Desk) -> Vec<Option<ChosenAccount>> {
    lock(&desk.held.0).clone()
}

/// Read from the file, because the stream folds a teammate's lines into one.
fn persona_lines(room: &Room) -> usize {
    std::fs::read_to_string(room.log.root().join("room.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| line.contains(r#""kind":"persona""#))
        .count()
}

fn anthropic(id: &str) -> Option<ChosenAccount> {
    Some(ChosenAccount {
        id: id.to_string(),
        provider_id: "anthropic".to_string(),
    })
}

#[tokio::test]
async fn a_hotline_switch_waits_for_the_turn_and_rebuilds_on_the_new_account() {
    let (scripted, gate) = gated();
    let desk = desk("hotline-switch-turn", scripted.clone(), false);
    let (room, b) = (&desk.room, desk.b.clone());
    started(room).await;
    mid_turn(room).await;
    assert_eq!(
        room.assign_account("ada", Some(b.clone())).await.unwrap(),
        SessionAccount {
            running: None,
            wanted: Some(b.clone()),
            held: None,
        }
    );
    assert_eq!(held(&desk), [None]);
    let chosen_on_the_record = persona_lines(room);
    gate.add_permits(10);
    until("switched", || running(room) == Some(b.clone())).await;
    assert_eq!(held(&desk), [None, anthropic(&b)]);
    assert_eq!(
        words(lock(&scripted.seeds).last().unwrap().clone()),
        [
            Said::User("Work on it.".to_string()),
            Said::Agent("Working.".to_string())
        ]
    );
    assert_eq!(persona_lines(room), chosen_on_the_record);
}

#[tokio::test]
async fn keeping_the_running_hotline_account_rebuilds_nothing() {
    let (scripted, gate) = gated();
    let desk = desk("hotline-switch-keep", scripted, true);
    let (room, a) = (&desk.room, desk.a.clone());
    started(room).await;
    mid_turn(room).await;
    room.assign_account("ada", Some(desk.b.clone()))
        .await
        .unwrap();
    assert_eq!(
        room.assign_account("ada", Some(a.clone())).await.unwrap(),
        switching(&a, &a, None)
    );
    gate.add_permits(10);
    until_state(room, "ada", SessionState::Ready).await;
    assert_eq!(held(&desk), [anthropic(&a)]);
}

#[tokio::test]
async fn going_from_no_choice_to_the_default_is_a_switch() {
    let desk = desk(
        "hotline-switch-default",
        Fake::new(Scripted::new(Vec::new())),
        false,
    );
    let (room, a) = (&desk.room, desk.a.clone());
    started(room).await;
    assert_eq!(
        room.assign_account("ada", Some(a.clone())).await.unwrap(),
        switching(&a, &a, None)
    );
    assert_eq!(held(&desk), [None, anthropic(&a)]);
}

#[tokio::test]
async fn a_hotline_choice_is_checked_against_the_teammates_model() {
    let desk = desk(
        "hotline-choice",
        Fake::new(Scripted::new(Vec::new())),
        false,
    );
    let (room, log) = (&desk.room, desk.room.log.clone());
    accounts(&log, 1);
    let openai = desk
        .vault
        .create("openai", "O", "not-a-real-key")
        .unwrap()
        .id;
    let gone = desk
        .vault
        .create("anthropic", "Gone", "not-a-real-key")
        .unwrap()
        .id;
    desk.vault.revoke(&gone).unwrap();
    let not_for_the_model = Err("That account is not for this teammate's model.".to_string());
    let assign = |account_id: &str| room.assign_account("ada", Some(account_id.to_string()));

    // The record's model over the room's, with the teammate idle.
    log.append(
        &StreamId::Room,
        &room::room_event(
            "setting",
            json!({"id": "defaultModelId", "value": "openai/gpt-x"}),
        ),
    )
    .unwrap();
    for (account_id, refusal) in [
        (
            openai.as_str(),
            "That account is not for this teammate's model.",
        ),
        (
            gone.as_str(),
            "That account is signed out. Sign in again first.",
        ),
        (
            "33333333-3333-4333-8333-333333333333",
            "That account is not on this desk.",
        ),
        (A, "That account is not on this desk."),
    ] {
        assert_eq!(
            assign(account_id).await,
            Err(refusal.to_string()),
            "{account_id}"
        );
    }
    assert_eq!(chosen(room), None);
    assign(&desk.a).await.unwrap();

    // No model on the record: the room's.
    let mut ada = room.persona("ada").unwrap();
    ada.model_id = None;
    room::append_persona(&log, &ada).unwrap();
    assert_eq!(assign(&desk.b).await, not_for_the_model);
    assign(&openai).await.unwrap();

    // The live session's model, over the record's and the room's.
    let mut ada = room.persona("ada").unwrap();
    ada.model_id = Some("openai/gpt-x".to_string());
    room::append_persona(&log, &ada).unwrap();
    started(room).await;
    assign(&desk.b).await.unwrap();
    room.set_model("ada", "openai/gpt-x").await.unwrap();
    assert_eq!(assign(&desk.a).await, not_for_the_model);
    assert_eq!(chosen(room), Some(desk.b.clone()));
}

#[tokio::test]
async fn a_pending_switch_survives_a_model_change_to_another_provider_and_back() {
    let (scripted, gate) = gated();
    let desk = desk("hotline-switch-model", scripted, true);
    let (room, a, b) = (&desk.room, desk.a.clone(), desk.b.clone());
    started(room).await;
    mid_turn(room).await;
    room.assign_account("ada", Some(b.clone())).await.unwrap();
    for model_id in ["openai/gpt-x", "anthropic/claude-x"] {
        room.set_model("ada", model_id).await.unwrap();
        assert_eq!(
            room.info("ada").account,
            Some(switching(&a, &b, None)),
            "{model_id}"
        );
    }
    gate.add_permits(10);
    until("switched", || running(room) == Some(b.clone())).await;
    assert_eq!(held(&desk), [anthropic(&a), anthropic(&b)]);
}
