//! The lead's `add_teammate`, through the real core on both drivers: one card
//! showing the teammate as it would be made, a yes that makes exactly that
//! with the room's defaults, a no that makes nothing, and no tool at all for
//! a teammate that does not lead.

use super::super::*;
use super::cards::{ask, said};
use super::live_work::Front;
use hotline_voice::realtime::{RealtimeEvent, RealtimeSession};
use std::sync::Mutex;

const RAE: &str = r#"{"name":"Rae","goal":"Research tide tables for the harbour."}"#;

fn roster(desk: &Desk) -> Vec<Value> {
    hotline_core::room::roster(&desk.log)
        .into_iter()
        .map(|persona| json!(persona))
        .collect()
}

fn pending_cards(desk: &Desk, persona: &str) -> usize {
    desk.log
        .load(&StreamId::Tape(persona.into()))
        .iter()
        .filter(|e| e["kind"] == "human_action" && e["status"] == "pending")
        .count()
}

/// The tool names on a teammate's ledger: what its session was built with.
async fn ledger(desk: &mut Desk, persona: &str) -> Vec<String> {
    let tools = desk
        .client
        .call("teammate.tools", json!({"personaId": persona}))
        .await;
    tools["result"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["name"].as_str().map(str::to_owned))
        .collect()
}

/// What a scripted turn's `RESULTS` says came back from `tool`, said or
/// thought: `<tool>: <text>` from Hotline Agent's script, `<tool> ok: <text>`
/// or `<tool> refused: <text>` from the ACP fixture.
async fn result_of(desk: &mut Desk, tape: i64, tool: &str) -> String {
    let line = |text: &str| {
        text.lines()
            .filter(|line| line.starts_with(tool))
            .find_map(|line| line.split_once(": ").map(|(_, said)| said.to_owned()))
    };
    let said = desk
        .event(tape, |e| {
            matches!(e["kind"].as_str(), Some("agent" | "thought"))
                && e["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("RESULTS") && line(text).is_some())
        })
        .await;
    line(said["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn the_leads_card_makes_the_teammate_only_on_yes_with_the_rooms_defaults() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        assert!(
            ledger(&mut desk, &ada)
                .await
                .contains(&"add_teammate".into())
        );

        let card = ask(
            &mut desk,
            &ada,
            "add_teammate",
            serde_json::from_str(RAE).unwrap(),
        )
        .await;
        let reason = card["reason"].as_str().unwrap().to_owned();
        assert!(
            reason.starts_with(
                "Add Rae as a teammate? Goal: Research tide tables for the harbour. Agent: "
            ),
            "{reason}"
        );
        assert!(reason.contains("Folder: "), "{reason}");
        assert_eq!(card["question"], true);
        assert_eq!(pending_cards(&desk, &ada), 1, "one ask, one card");
        assert_eq!(roster(&desk).len(), 1, "nothing exists before the yes");

        let declined = desk
            .client
            .call(
                "human.answer",
                json!({"personaId":ada,"actionId":card["actionId"],"status":"declined"}),
            )
            .await;
        assert_eq!(declined["ok"], true, "{declined}");
        assert_eq!(roster(&desk).len(), 1, "a no makes nothing");

        let card = ask(
            &mut desk,
            &ada,
            "add_teammate",
            serde_json::from_str(RAE).unwrap(),
        )
        .await;
        let approved = desk
            .client
            .call(
                "human.answer",
                json!({"personaId":ada,"actionId":card["actionId"],"status":"done"}),
            )
            .await;
        assert_eq!(approved["ok"], true, "{approved}");
        let rae = roster(&desk)
            .into_iter()
            .find(|persona| persona["name"] == "Rae")
            .expect("the yes made Rae");
        assert_eq!(rae["goal"], "Research tide tables for the harbour.");
        let rae_id = rae["id"].as_str().unwrap().to_owned();
        let workspace = hotline_core::paths::default_workspace(desk.root.path(), &rae_id);
        assert_eq!(rae["cwd"], json!(workspace.to_string_lossy()));
        assert!(
            card["reason"]
                .as_str()
                .unwrap()
                .ends_with(&format!("Folder: {}.", workspace.display())),
            "the card showed the folder the yes made: {}",
            card["reason"]
        );
        assert!(rae.get("reach").is_none(), "{rae}");
        assert_eq!(rae["mcpPolicy"], json!({"mode":"none","serverIds":[]}));
        assert_eq!(rae["backgroundWork"], false);
        // The pane's own create with nothing named takes the same defaults.
        let pane = desk
            .client
            .call("persona.create", json!({"draft": {"name": "Pane"}}))
            .await;
        assert_eq!(rae["backendId"], pane["result"]["backendId"]);
        assert_eq!(rae.get("modelId"), pane["result"].get("modelId"));
        let again = desk
            .client
            .call(
                "human.answer",
                json!({"personaId":ada,"actionId":card["actionId"],"status":"done"}),
            )
            .await;
        assert_eq!(again["ok"], false);
        let named_rae = roster(&desk).iter().filter(|p| p["name"] == "Rae").count();
        assert_eq!(named_rae, 1, "a second yes makes no second teammate");

        // The lead reaches the new teammate by name straight away.
        let sent = desk
            .client
            .call(
                "session.prompt",
                json!({"personaId":ada,"text":r#"CALL message_teammate {"to":"Rae","message":"Welcome aboard."}"#}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
        let result = result_of(&mut desk, tape, "message_teammate").await;
        assert!(result.contains(r#""sent":true,"to":"Rae""#), "{result}");
    }
}

#[tokio::test]
async fn a_teammate_that_does_not_lead_has_no_add_teammate() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, ada_tape) = desk.teammate("Ada", driver, false).await;
        let (bob, bob_tape) = desk.teammate("Bob", driver, false).await;
        assert!(
            !ledger(&mut desk, &bob)
                .await
                .contains(&"add_teammate".into())
        );

        let asked = desk
            .client
            .call(
                "session.prompt",
                json!({"personaId":bob,"text":format!("CALL add_teammate {RAE}")}),
            )
            .await;
        assert_eq!(asked["ok"], true, "{asked}");
        let refused = result_of(&mut desk, bob_tape, "add_teammate").await;
        assert!(!refused.contains("Asked."), "{refused}");
        assert_eq!(pending_cards(&desk, &bob), 0);
        assert_eq!(roster(&desk).len(), 2);

        // The role moves while Ada's session is live. Hotline Agent's tools
        // follow the roster and drop it; the ACP child calls it anyway, and
        // the call is checked again and refused.
        let moved = desk.client.call("persona.lead", json!({"id": bob})).await;
        assert_eq!(moved["ok"], true, "{moved}");
        let asked = desk
            .client
            .call(
                "session.prompt",
                json!({"personaId":ada,"text":format!("CALL add_teammate {RAE}")}),
            )
            .await;
        assert_eq!(asked["ok"], true, "{asked}");
        let refused = result_of(&mut desk, ada_tape, "add_teammate").await;
        assert!(!refused.contains("Asked."), "{refused}");
        if matches!(driver, Driver::Acp) {
            assert!(refused.contains("Only the team lead"), "{refused}");
        }
        assert_eq!(pending_cards(&desk, &ada), 0);
        assert_eq!(roster(&desk).len(), 2);
    }
}

#[tokio::test]
async fn a_spoken_yes_on_a_call_adds_the_teammate() {
    let (session, mut asked, events) = RealtimeSession::scripted(64);
    let front = Arc::new(Front {
        provider: "openai",
        session: Mutex::new(Some(session)),
        config: Mutex::new(None),
    });
    let mut desk = open_with_front(Some(front.clone())).await;
    let (ada, _) = desk.teammate("Ada", Driver::HotlineAgent, false).await;
    let call = uuid::Uuid::new_v4().to_string();
    let result = desk
        .client
        .call(
            "voice.call_start",
            json!({"callId":call,"personaId":ada,"mode":"front"}),
        )
        .await;
    assert_eq!(result["ok"], true, "{result}");
    crate::common::until("the live front", || front.config.lock().unwrap().is_some()).await;

    let card = ask(
        &mut desk,
        &ada,
        "add_teammate",
        serde_json::from_str(RAE).unwrap(),
    )
    .await;
    assert!(said(&mut asked).await.contains("Add Rae as a teammate?"));
    events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
    events
        .send(Ok(RealtimeEvent::Heard("yes".into())))
        .await
        .unwrap();
    crate::common::until("the teammate card settles", || {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .iter()
            .any(|e| e["id"] == card["id"] && e["status"] == "done")
    })
    .await;
    assert!(roster(&desk).iter().any(|persona| persona["name"] == "Rae"));
    desk.client
        .call("voice.call_end", json!({"callId":call}))
        .await;
}
