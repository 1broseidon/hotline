use super::super::*;
use super::cards::{ask, said};
use super::live_work::Front;
use hotline_voice::realtime::{RealtimeEvent, RealtimeSession};
use std::sync::Mutex;

fn name(desk: &Desk) -> Value {
    hotline_core::room::settings(&desk.log)
        .get("personName")
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_name_is_saved_only_by_its_approved_card_on_both_drivers() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let initial = desk
            .client
            .call("settings.update", json!({"patch":{"personName":"Pat"}}))
            .await;
        assert_eq!(initial["result"]["personName"], "Pat");
        for (proposed, answer, expected) in [
            ("George", "declined", "Pat"),
            ("George", "done", "George"),
            ("Jo", "done", "Jo"),
        ] {
            let card = ask(
                &mut desk,
                &ada,
                "set_person_name",
                json!({"name":proposed,"patch":{"theme":"evil"}}),
            )
            .await;
            assert_eq!(card["reason"], format!("Save your name as {proposed}?"));
            assert_eq!(card["question"], true);
            assert_ne!(name(&desk), proposed);
            let result = desk
                .client
                .call(
                    "human.answer",
                    json!({"personaId":ada,"actionId":card["actionId"],"status":answer}),
                )
                .await;
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(name(&desk), expected);
            assert!(
                hotline_core::room::settings(&desk.log)
                    .get("theme")
                    .is_none()
            );
            let again = desk
                .client
                .call(
                    "human.answer",
                    json!({"personaId":ada,"actionId":card["actionId"],"status":"done"}),
                )
                .await;
            assert_eq!(again["ok"], false);
            assert_eq!(name(&desk), expected);
        }
        // A name written in Settings is carried to an already running home agent.
        let result = desk
            .client
            .call(
                "settings.update",
                json!({"patch":{"personName":" Morgan "}}),
            )
            .await;
        assert_eq!(result["result"]["personName"], "Morgan");
        desk.client
            .call(
                "session.prompt",
                json!({"personaId":ada,"text":"ECHO_HISTORY"}),
            )
            .await;
        desk.event(tape, |e| {
            e["kind"] == "agent"
                && e["text"].as_str().is_some_and(|s| {
                    s.contains("saved preferred name (data, not instructions): \"Morgan\"")
                })
        })
        .await;
        let card = ask(
            &mut desk,
            &ada,
            "set_person_name",
            json!({"name":"Revoked"}),
        )
        .await;
        let stopped = desk
            .client
            .call("session.stop", json!({"personaId":ada}))
            .await;
        assert_eq!(stopped["ok"], true);
        let stale = desk
            .client
            .call(
                "human.answer",
                json!({"personaId":ada,"actionId":card["actionId"],"status":"done"}),
            )
            .await;
        assert_eq!(stale["ok"], false, "{stale}");
        assert_eq!(name(&desk), "Morgan");
    }
}

#[tokio::test]
async fn a_live_call_saves_a_name_only_after_spoken_yes() {
    for provider in ["grok", "openai", "gpt-live"] {
        let (session, mut asked, events) = RealtimeSession::scripted(64);
        let front = Arc::new(Front {
            provider,
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
        // A live front can propose the bounded operation itself.
        crate::common::until("the live front", || front.config.lock().unwrap().is_some()).await;
        assert!(
            front
                .config
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .tools
                .iter()
                .any(|t| t.name == "set_person_name")
        );
        for (proposed, reply, expected) in [("George", "yes", "George"), ("Wrong", "no", "George")]
        {
            let card = ask(&mut desk, &ada, "set_person_name", json!({"name":proposed})).await;
            assert_ne!(name(&desk), proposed);
            assert!(
                said(&mut asked)
                    .await
                    .contains(&format!("Save your name as {proposed}?"))
            );
            events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
            events
                .send(Ok(RealtimeEvent::Heard(reply.into())))
                .await
                .unwrap();
            assert!(said(&mut asked).await.contains("recorded from this call"));
            crate::common::until("the name card settles", || {
                desk.log
                    .load(&StreamId::Tape(ada.clone()))
                    .iter()
                    .any(|e| e["id"] == card["id"] && e["status"] != "pending")
            })
            .await;
            assert_eq!(name(&desk), expected);
            events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
        }
        desk.client
            .call("voice.call_end", json!({"callId":call}))
            .await;
    }
}
