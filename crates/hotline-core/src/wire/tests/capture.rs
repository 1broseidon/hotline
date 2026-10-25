//! The phone's only value-bearing secret command answers a bound live card.
use super::*;
use crate::contract::{CredentialAnswer, CredentialValues, PersonaComputer};
use crate::driver::CapabilityEpoch;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona};

#[tokio::test]
async fn capture_phone_answer_settles_both_seats_once_and_other_value_commands_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let mut ada = persona("ada");
    ada.name = "Pork Chop".into();
    ada.computer = Some(PersonaComputer {
        enabled: true,
        cpus: None,
        memory: None,
        pids: None,
        image: None,
        mounts: None,
        secrets: None,
    });
    enrol(&log, &ada);
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(
            root.path(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let room = Room::with_agents_computers_and_vault(
        log.clone(),
        Arc::new(DeskKeys),
        Fake::new(Scripted::turns(vec![vec![]])),
        crate::computer::Computer::new(),
        Some(vault.clone()),
    );
    let desk: Arc<dyn RoomHandle> =
        Arc::new(crate::desk::Desk::with_room(log.clone(), room.clone()));
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    TeammateTools::new(&room,"ada").with_capability(CapabilityEpoch::default().lease()).call("request_credential",&json!({"site":"https://github.com","kind":"login","name":"GITHUB_LOGIN","reason":"Sign in to the repository"})).await.unwrap();
    let pending = log
        .load(&StreamId::Tape("ada".into()))
        .into_iter()
        .find(|e| e["kind"] == "credential_ask")
        .unwrap();
    let action_id = pending["actionId"].as_str().unwrap();
    let submitted = CredentialAnswer {
        action_id: action_id.into(),
        name: Some("GITHUB_LOGIN".into()),
        values: Some(CredentialValues {
            username: "owner-name".into(),
            password: "only-for-the-vault".into(),
            totp: None,
        }),
        ask: false,
    };
    for seat in [Seat::Owner, Seat::Phone] {
        let snapshot = super::asks::seat_snapshot(seat, &desk, &log, json!({"tape":"ada"})).await;
        assert!(
            snapshot
                .iter()
                .any(|e| e["id"] == pending["id"] && e["status"] == "pending")
        );
    }
    for frame in [
        json!({"id":1,"cmd":"secrets.set","params":{"name":"OTHER","value":"private-value"}}),
        json!({"id":1,"cmd":"secrets.login.set","params":{"name":"OTHER","sites":["https://other.com"],"username":"person","password":"private-value"}}),
        json!({"id":1,"cmd":"thread.answer","params":{"thread":{"kind":"dm","key":"ada"},"answer":{"kind":"credential","actionId":action_id,"name":"GITHUB_LOGIN","values":{"username":"person","password":"private-value"},"ask":false}}}),
    ] {
        let reply = remote_control_answer(Seat::Phone, &desk, &log, &Parts::default(), frame).await;
        assert_eq!(reply["ok"], false, "{reply}");
    }
    let frame = json!({"id":2,"cmd":"secrets.capture.answer","params":submitted});
    let reply =
        remote_control_answer(Seat::Phone, &desk, &log, &Parts::default(), frame.clone()).await;
    assert_eq!(reply["ok"], true, "{reply}");
    let again = remote_control_answer(Seat::Owner, &desk, &log, &Parts::default(), frame).await;
    assert_eq!(again["ok"], false);
    for seat in [Seat::Owner, Seat::Phone] {
        let snapshot = super::asks::seat_snapshot(seat, &desk, &log, json!({"tape":"ada"})).await;
        assert!(snapshot.iter().any(|e| e["id"] == pending["id"]
            && e["status"] == "stored"
            && e["storedAs"] == "GITHUB_LOGIN"));
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("only-for-the-vault")
        );
    }
    assert_eq!(vault.shared_secrets().unwrap().len(), 1);
    assert_eq!(
        crate::room::roster(&log)[0]
            .computer
            .as_ref()
            .unwrap()
            .secrets
            .as_ref()
            .unwrap(),
        &vec!["GITHUB_LOGIN".to_string()]
    );
}

#[test]
fn capture_malformed_values_and_debug_never_echo_typed_values() {
    for params in [
        json!({"actionId":"missing","name":"KEY","values":{"username":"someone","password":["SECRET_SENTINEL"]}}),
        json!({"actionId":"missing","name":"KEY","values":{"username":"someone","password":"password","totp":{"SECRET_SENTINEL":true}}}),
        json!({"actionId":"missing","values":{},"site":"https://attacker.example"}),
    ] {
        let error = super::super::socket::read_command(
            &json!({"cmd":"secrets.capture.answer","params":params}),
        )
        .unwrap_err();
        assert!(!error.contains("SECRET_SENTINEL"));
    }
}
