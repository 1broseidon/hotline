//! Capture through the same tool and socket as a teammate and an owner.
use super::*;
use crate::contract::{CredentialAnswer, CredentialValues, ThreadKind};
use futures_util::{SinkExt, StreamExt};

fn request(kind: &str) -> Value {
    json!({"site":"https://github.com", "reason":"Sign in to work on the repository", "name":"GITHUB_LOGIN", "kind":kind})
}
fn card(room: &Room) -> Value {
    tape(room, "ada")
        .into_iter()
        .rev()
        .find(|e| e["kind"] == "credential_ask")
        .unwrap()
}
fn answer(id: &str) -> CredentialAnswer {
    CredentialAnswer {
        action_id: id.into(),
        name: Some("CHOSEN_LOGIN".into()),
        ask: false,
        values: Some(CredentialValues {
            username: "capture-user-sentinel".into(),
            password: "capture-password-sentinel".into(),
            totp: Some("JBSWY3DPEHPK3PXP".into()),
        }),
    }
}
fn tools(room: &Arc<Room>, lease: CapabilityLease) -> TeammateTools {
    TeammateTools::new(room, "ada").with_capability(lease)
}
async fn delivered(room: &Room, needle: &str) -> String {
    until("the credential outcome reaches its agent", || {
        tape(room, "ada").iter().any(|e| {
            e["kind"] == "delivery" && e["text"].as_str().is_some_and(|t| t.contains(needle))
        })
    })
    .await;
    tape(room, "ada")
        .iter()
        .filter(|e| e["kind"] == "delivery")
        .map(|e| e["text"].as_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn capture_stores_grants_and_delivers_without_values_in_any_sink() {
    let ComputerRoom {
        room,
        taken,
        agents,
        ..
    } = computer_room("capture-login", Some("0.10.3"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    let tool = tools(&room, CapabilityEpoch::default().lease());
    let asked = tool
        .call("request_credential", &request("login"))
        .await
        .unwrap();
    let pending = card(&room);
    let id = pending["actionId"].as_str().unwrap();
    assert!(asked.contains(id));
    assert_eq!(pending["site"], "https://github.com");
    assert!(crate::wire::waiting_on(&tape(&room, "ada")));

    let desk = Arc::new(crate::desk::Desk::with_room(room.log.clone(), room.clone()));
    let door = crate::wire::Door::bind(
        room.log.clone(),
        "capture-token".into(),
        desk,
        crate::wire::Parts::default(),
    )
    .unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?token=capture-token"))
            .await
            .unwrap();
    let submitted = answer(id);
    let debug = format!(
        "{:?}",
        crate::contract::Command::SecretsCaptureAnswer(submitted.clone())
    );
    socket
        .send(tokio_tungstenite::tungstenite::Message::text(
            json!({"id":1,"cmd":"secrets.capture.answer","params":submitted}).to_string(),
        ))
        .await
        .unwrap();
    let reply = socket
        .next()
        .await
        .unwrap()
        .unwrap()
        .to_text()
        .unwrap()
        .to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&reply).unwrap()["ok"],
        true,
        "{reply}"
    );
    assert!(
        room.answer_credential(answer(id)).await.is_err(),
        "a second seat cannot answer again"
    );
    assert_eq!(card(&room)["status"], "stored");
    assert_eq!(card(&room)["storedAs"], "CHOSEN_LOGIN");
    let outcome = delivered(&room, "Stored as CHOSEN_LOGIN").await;
    assert!(outcome.contains("https://github.com"));
    assert!(outcome.contains("browser fill secret CHOSEN_LOGIN.password"));
    let persona = room.persona("ada").unwrap();
    assert_eq!(
        persona.computer.unwrap().secrets.unwrap(),
        vec!["CHOSEN_LOGIN"]
    );
    let sets = taken.sets();
    let login = &sets.last().unwrap()["CHOSEN_LOGIN"];
    assert_eq!(login["sites"], json!(["https://github.com"]));
    assert_eq!(login["password"], "capture-password-sentinel");
    assert_eq!(login["ask"], false);
    assert_eq!(
        room.vault.as_ref().unwrap().shared_secrets().unwrap()[0].sites,
        Some(vec!["https://github.com".into()])
    );
    let transcript = serde_json::to_string(&tape(&room, "ada")).unwrap();
    let preambles = format!("{:?}", lock(&agents.preambles));
    for sink in [&reply, &debug, &outcome, &transcript, &preambles] {
        for value in [
            "capture-user-sentinel",
            "capture-password-sentinel",
            "JBSWY3DPEHPK3PXP",
        ] {
            assert!(!sink.contains(value), "typed value leaked");
        }
    }
    // Inspect every durable stream, including raw superseded tape lines.
    fn scan(path: &std::path::Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == "vault" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                scan(&entry.path());
            } else if let Ok(bytes) = std::fs::read(entry.path()) {
                let text = String::from_utf8_lossy(&bytes);
                for value in [
                    "capture-user-sentinel",
                    "capture-password-sentinel",
                    "JBSWY3DPEHPK3PXP",
                ] {
                    assert!(
                        !text.contains(value),
                        "typed value in {}",
                        entry.path().display()
                    );
                }
            }
        }
    }
    scan(room.log.root());
    serving.abort();
}

#[tokio::test]
async fn capture_decline_expiry_withdrawal_and_revocation_store_nothing() {
    for outcome in ["declined", "expired", "withdrawn", "revoked"] {
        let ComputerRoom { room, taken, .. } = computer_room(
            &format!("capture-{outcome}"),
            Some("0.10.3"),
            TWO_RELEASES,
            true,
        )
        .await;
        let _turn = room.open_posting(&ThreadId::dm("ada"), None);
        let epoch = CapabilityEpoch::default();
        let tool = tools(&room, epoch.lease());
        tool.call("request_credential", &request("login"))
            .await
            .unwrap();
        let id = card(&room)["actionId"].as_str().unwrap().to_string();
        match outcome {
            "declined" => {
                room.answer_credential(CredentialAnswer {
                    action_id: id.clone(),
                    name: None,
                    values: None,
                    ask: false,
                })
                .await
                .unwrap();
                assert!(
                    tool.call("request_credential", &request("login"))
                        .await
                        .unwrap_err()
                        .contains("Already asked")
                );
            }
            "expired" => {
                assert!(!room.expire_credential(&id, now_ms() + 31 * 60_000).await);
            }
            "withdrawn" => {
                tool.call("withdraw_request", &json!({"id":id}))
                    .await
                    .unwrap();
            }
            _ => {
                epoch.stop();
                assert!(room.answer_credential(answer(&id)).await.is_err());
            }
        }
        assert_eq!(
            card(&room)["status"],
            if outcome == "revoked" {
                "expired"
            } else {
                outcome
            }
        );
        assert!(room.answer_credential(answer(&id)).await.is_err());
        assert_eq!(
            room.vault.as_ref().unwrap().shared_secrets().unwrap(),
            Vec::<crate::contract::SharedSecret>::new()
        );
        assert!(
            room.persona("ada")
                .unwrap()
                .computer
                .unwrap()
                .secrets
                .is_none()
        );
        assert_eq!(
            taken.sets(),
            Vec::<std::collections::BTreeMap<String, Value>>::new()
        );
        if outcome != "revoked" {
            let text = delivered(&room, "Nothing was stored").await;
            assert!(text.contains(outcome));
        }
    }
}

#[tokio::test]
async fn capture_existing_key_is_offered_only_for_its_origin_and_cannot_be_replaced() {
    let ComputerRoom { room, .. } =
        computer_room("capture-existing", Some("0.10.3"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared(
            "OLD",
            crate::vault::StoredSecret::Login {
                sites: vec!["https://github.com".into()],
                username: "existing".into(),
                password: "existing-password".into(),
                totp: None,
                ask: true,
            },
        )
        .unwrap();
    vault
        .set_shared(
            "OTHER",
            crate::vault::StoredSecret::Login {
                sites: vec!["https://other.com".into()],
                username: "other".into(),
                password: "other-password".into(),
                totp: None,
                ask: false,
            },
        )
        .unwrap();
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    tools(&room, CapabilityEpoch::default().lease())
        .call("request_credential", &request("login"))
        .await
        .unwrap();
    let pending = card(&room);
    assert_eq!(pending["existing"], json!(["OLD"]));
    let mut grant = CredentialAnswer {
        action_id: pending["actionId"].as_str().unwrap().into(),
        name: Some("OTHER".into()),
        values: None,
        ask: false,
    };
    assert!(room.answer_credential(grant.clone()).await.is_err());
    grant.name = Some("OLD".into());
    room.answer_credential(grant).await.unwrap();
    assert_eq!(card(&room)["storedAs"], "OLD");
    assert!(
        vault
            .shared_secrets()
            .unwrap()
            .iter()
            .find(|s| s.name == "OLD")
            .unwrap()
            .ask
    );
}

#[tokio::test]
async fn capture_passkey_keeps_approval_origin_name_and_ask_setting() {
    let ComputerRoom { room, taken, .. } =
        computer_room("capture-passkey", Some("0.10.3"), TWO_RELEASES, true).await;
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    tools(&room, CapabilityEpoch::default().lease())
        .call("request_credential", &request("passkey"))
        .await
        .unwrap();
    let id = card(&room)["actionId"].as_str().unwrap().to_string();
    assert!(
        room.answer_credential(answer(&id)).await.is_err(),
        "a passkey takes no values"
    );
    room.answer_credential(CredentialAnswer {
        action_id: id,
        name: Some("GH_PASSKEY".into()),
        values: None,
        ask: true,
    })
    .await
    .unwrap();
    assert_eq!(card(&room)["armed"], true);
    assert_eq!(taken.armed().as_deref(), Some("github.com"));
    let ask = taken.ask();
    let state = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(
        state.state,
        crate::contract::PasskeyRegistrationState::Asked
    );
    assert_eq!(
        room.vault.as_ref().unwrap().shared_secrets().unwrap(),
        Vec::<crate::contract::SharedSecret>::new()
    );
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    taken.mint();
    room.secrets_passkey_registration("ada").await.unwrap();
    until("the credential card settles", || {
        card(&room)["status"] == "stored"
    })
    .await;
    assert_eq!(card(&room)["storedAs"], "GH_PASSKEY");
    let secret = room
        .vault
        .as_ref()
        .unwrap()
        .shared_secrets()
        .unwrap()
        .remove(0);
    assert!(secret.ask);
    assert_eq!(
        super::super::secrets::describe_secret(&secret),
        "a passkey for github.com"
    );
    assert!(
        !serde_json::to_string(&tape(&room, "ada"))
            .unwrap()
            .contains(crate::computer::guide::fake::KEY_BASE64)
    );
    assert_eq!(taken.sets().last().unwrap()["GH_PASSKEY"]["ask"], true);
}

#[tokio::test]
async fn capture_passkey_unused_arming_expires_and_call_card_stays_on_the_desk() {
    let ComputerRoom { room, .. } = computer_room(
        "capture-passkey-expired",
        Some("0.10.3"),
        TWO_RELEASES,
        true,
    )
    .await;
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    tools(&room, CapabilityEpoch::default().lease())
        .call("request_credential", &request("passkey"))
        .await
        .unwrap();
    let id = card(&room)["actionId"].as_str().unwrap().to_string();
    room.answer_credential(CredentialAnswer {
        action_id: id.clone(),
        name: Some("GH_PASSKEY".into()),
        values: None,
        ask: false,
    })
    .await
    .unwrap();
    assert!(!room.expire_credential(&id, now_ms() + 11 * 60_000).await);
    assert_eq!(card(&room)["status"], "expired");
    assert_eq!(
        room.vault.as_ref().unwrap().shared_secrets().unwrap(),
        Vec::<crate::contract::SharedSecret>::new()
    );

    // A call's card is core-stamped into home chat; spoken yes has no authority.
    let call = ThreadId {
        kind: ThreadKind::Call,
        key: "capture-call".into(),
    };
    let _turn = room.open_posting(&call, None);
    let tool = tools(&room, CapabilityEpoch::default().lease()).for_call(call.clone());
    tool.call("request_credential", &request("login"))
        .await
        .unwrap();
    let pending = card(&room);
    assert_eq!(pending["from"]["kind"], "call");
    assert_eq!(pending["from"]["thread"], call.key);
    let id = pending["actionId"].as_str().unwrap();
    let reference = hotline_voice::cards::Card {
        persona: "ada".into(),
        kind: "credential_ask".into(),
        request: id.into(),
    };
    let notice = room.call_card_notice(&reference, true).unwrap();
    assert!(!notice.answerable);
    assert!(notice.text.contains("desk and phone"));
    assert!(!notice.text.contains("Sign in"));
    room.answer_credential(answer(id)).await.unwrap();
    assert_eq!(card(&room)["status"], "stored");
}

#[tokio::test]
async fn capture_passkey_refuses_another_origin_and_minting_without_operator_approval() {
    for case in [
        "capture-other-origin",
        "capture-unapproved",
        "capture-replaced-request",
    ] {
        let another_origin = case == "capture-other-origin";
        let ComputerRoom { room, taken, .. } =
            computer_room(case, Some("0.10.3"), TWO_RELEASES, true).await;
        let _turn = room.open_posting(&ThreadId::dm("ada"), None);
        let mut request = request("passkey");
        if another_origin {
            request["site"] = json!("https://github.com:8443");
        }
        tools(&room, CapabilityEpoch::default().lease())
            .call("request_credential", &request)
            .await
            .unwrap();
        let id = card(&room)["actionId"].as_str().unwrap().to_string();
        room.answer_credential(CredentialAnswer {
            action_id: id.clone(),
            name: Some("GH_PASSKEY".into()),
            values: None,
            ask: false,
        })
        .await
        .unwrap();
        let ask = taken.ask();
        if case == "capture-replaced-request" {
            room.secrets_passkey_registration("ada").await.unwrap();
            room.secrets_passkey_answer("ada", &ask, true)
                .await
                .unwrap();
            taken.ask();
        }
        if !another_origin {
            taken.mint();
        }
        // Either this look or the room's watcher may reject the credential first.
        let _ = room.secrets_passkey_registration("ada").await;
        assert!(!room.expire_credential(&id, now_ms()).await);
        assert_eq!(card(&room)["status"], "expired");
        assert_eq!(
            room.vault.as_ref().unwrap().shared_secrets().unwrap(),
            Vec::<crate::contract::SharedSecret>::new()
        );
        assert!(
            room.persona("ada")
                .unwrap()
                .computer
                .unwrap()
                .secrets
                .is_none()
        );
        // Waking the agent may hand its computer the existing empty set.
        for set in taken.sets() {
            assert_eq!(set, std::collections::BTreeMap::new());
        }
        assert!(taken.answered().is_none());
    }
}

#[tokio::test]
async fn capture_an_accepted_answer_finishes_when_its_phone_disconnects() {
    let ComputerRoom { room, taken, .. } =
        computer_room("capture-disconnect", Some("0.10.3"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    tools(&room, CapabilityEpoch::default().lease())
        .call("request_credential", &request("login"))
        .await
        .unwrap();
    let id = card(&room)["actionId"].as_str().unwrap().to_string();
    let capture = lock(&room.captures).get(&id).cloned().unwrap();
    let updates = room.policy_update_lock();
    let held = updates.lock().await;
    let command = {
        let room = room.clone();
        tokio::spawn(async move { room.answer_credential(answer(&id)).await })
    };
    until("the accepted command waits to commit", || {
        capture.try_lock().is_err()
    })
    .await;
    command.abort();
    drop(held);
    until("the disconnected phone's answer settles", || {
        card(&room)["status"] == "stored"
    })
    .await;
    assert_eq!(
        taken.sets().last().unwrap()["CHOSEN_LOGIN"]["sites"],
        json!(["https://github.com"])
    );
    assert_eq!(
        room.persona("ada")
            .unwrap()
            .computer
            .unwrap()
            .secrets
            .unwrap(),
        vec!["CHOSEN_LOGIN"]
    );
}

#[tokio::test]
async fn capture_cannot_widen_a_job_without_a_computer_or_a_voice_front_or_run() {
    let ComputerRoom { room, .. } =
        computer_room("capture-narrowed", Some("0.10.3"), TWO_RELEASES, true).await;
    let job: ScheduledJob = serde_json::from_value(json!({
        "id":"without-computer", "personaId":"ada", "kind":"schedule",
        "prompt":"Work without a computer", "without":["computer"],
        "nextAt":now_ms()+60_000, "createdAt":now_ms(), "endedAt":now_ms(),
    }))
    .unwrap();
    room::append_schedule(&room.log, &job).unwrap();
    let home = ThreadId::job(&job.id);
    let _turn = room.open_posting(&home, None);
    for tool in [
        tools(&room, CapabilityEpoch::default().lease()).for_job(&job.id),
        tools(&room, CapabilityEpoch::default().lease()).for_front(),
        tools(&room, CapabilityEpoch::default().lease()).for_run(),
        TeammateTools::new(&room, "ada"),
    ] {
        assert!(
            tool.call("request_credential", &request("login"))
                .await
                .is_err()
        );
    }
    assert!(
        !tape(&room, "ada")
            .iter()
            .any(|e| e["kind"] == "credential_ask")
    );
    assert_eq!(
        room.vault.as_ref().unwrap().shared_secrets().unwrap(),
        Vec::<crate::contract::SharedSecret>::new()
    );
}
