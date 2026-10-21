use super::*;
use crate::mcp::server::TeammateTools;
use hotline_tools::capability::CapabilityEpoch;
use hotline_voice::cards::Card;

fn pending(desk: &crate::desk::Desk, persona: &str) -> Value {
    desk.log
        .load(&StreamId::Tape(persona.into()))
        .into_iter()
        .rev()
        .find(|e| e["kind"] == "human_action" && e["status"] == "pending")
        .unwrap()
}

fn reference(persona: &str, event: &Value) -> Card {
    Card {
        persona: persona.into(),
        kind: "human_action".into(),
        request: event["actionId"].as_str().unwrap().into(),
    }
}

#[tokio::test]
async fn the_card_handler_refuses_actions_forged_questions_and_stale_authority() {
    let (_root, desk, calls, call, persona, _events, _fake) = direct_call().await;
    let room = desk.test_room();
    let epoch = CapabilityEpoch::default();
    let tools = TeammateTools::new(&room, persona.clone()).with_capability(epoch.lease());
    for reason in [
        "Enter a credential",
        "Tap the 2FA prompt",
        "Look at the screen",
        "Make a payment",
        "Delete the database",
    ] {
        // A model cannot promote an action by adding a voice/question flag.
        tools
            .call(
                "request_human",
                &json!({"reason":reason,"question":true,"voice":true}),
            )
            .await
            .unwrap();
        let event = pending(&desk, &persona);
        let card = reference(&persona, &event);
        assert!(event.get("question").is_none());
        let desk_notice = room.call_card_notice(&card, false).unwrap();
        assert!(!desk_notice.answerable);
        // The desk's voice names the teammate; the teammate's own voice says I.
        assert!(
            !desk_notice.text.starts_with("I ")
                && desk_notice
                    .text
                    .contains(&format!(" needs you for an action: {reason}. "))
        );
        assert!(
            room.call_card_notice(&card, true)
                .unwrap()
                .text
                .starts_with(&format!("I need you for an action: {reason}. "))
        );
        for yes in [true, false] {
            assert!(
                room.answer_call_card(&card, &call, yes)
                    .await
                    .unwrap_err()
                    .contains("desk")
            );
        }
        assert_eq!(pending(&desk, &persona)["id"], event["id"]);
        room.withdraw_human(&persona, &card.request).unwrap();
    }
    for kind in ["permission", "passkey_ask"] {
        let card = Card {
            persona: persona.clone(),
            kind: kind.into(),
            request: "collab:forged".into(),
        };
        assert!(room.answer_call_card(&card, &call, true).await.is_err());
    }
    tools
        .call(
            "ask_question",
            &json!({"question":"Is blue your preferred colour?"}),
        )
        .await
        .unwrap();
    let card = reference(&persona, &pending(&desk, &persona));
    assert!(room.call_card_notice(&card, false).unwrap().answerable);
    let mut wrong = card.clone();
    wrong.persona = "someone-else".into();
    assert!(room.answer_call_card(&wrong, &call, true).await.is_err());
    calls.hold(&call, true).unwrap();
    assert!(room.answer_call_card(&card, &call, true).await.is_err());
    calls.hold(&call, false).unwrap();
    epoch.stop();
    assert!(
        room.answer_call_card(&card, &call, true)
            .await
            .unwrap_err()
            .contains("revoked")
    );
    assert!(!room.call_card_notice(&card, false).unwrap().answerable);
    assert_eq!(pending(&desk, &persona)["actionId"], card.request);
    calls.end(&call).unwrap();
    assert!(room.answer_call_card(&card, &call, false).await.is_err());
}

#[tokio::test]
async fn duplicate_cards_are_said_once_and_withdrawn_cards_are_not_said() {
    let (_root, desk, calls, call, persona, mut events, fake) = direct_call().await;
    let room = desk.test_room();
    calls.hold(&call, true).unwrap();
    room.ask_human(
        &persona,
        "Check the screen",
        &crate::contract::ThreadId::dm(&persona),
        None,
    )
    .unwrap();
    let old = pending(&desk, &persona);
    room.withdraw_human(&persona, old["actionId"].as_str().unwrap())
        .unwrap();
    room.ask_human(
        &persona,
        "Enter credentials",
        &crate::contract::ThreadId::dm(&persona),
        None,
    )
    .unwrap();
    let card = pending(&desk, &persona);
    calls.card(&persona, &card);
    calls.card(&persona, &card);
    calls.hold(&call, false).unwrap();
    event(&mut events, |e| {
        matches!(e, VoiceEvent::Clip { r#final: true, .. })
    })
    .await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        lock(&fake.spoken)
            .iter()
            .filter(|text| text.starts_with("I need you for an action"))
            .count(),
        1
    );
    calls.end(&call).unwrap();
}
