use super::super::*;
use super::live_work::Front;
use hotline_voice::realtime::{Command, RealtimeEvent, RealtimeSession};
use std::sync::Mutex;

async fn ask(desk: &mut Desk, persona: &str, tool: &str, arguments: Value) -> Value {
    let result = desk
        .client
        .call(
            "session.prompt",
            json!({
                "personaId":persona, "text":format!("CALL {tool} {arguments}")
            }),
        )
        .await;
    assert_eq!(result["ok"], true, "{result}");
    crate::common::until("the pending card", || {
        desk.log
            .load(&StreamId::Tape(persona.into()))
            .iter()
            .any(|e| e["kind"] == "human_action" && e["status"] == "pending")
    })
    .await;
    desk.log
        .load(&StreamId::Tape(persona.into()))
        .into_iter()
        .find(|e| e["kind"] == "human_action" && e["status"] == "pending")
        .unwrap()
}

async fn said(asked: &mut tokio::sync::mpsc::Receiver<Command>) -> String {
    tokio::time::timeout(PATIENCE, async {
        loop {
            if let Some(Command::Say(text)) = asked.recv().await {
                return text;
            }
        }
    })
    .await
    .expect("the card is said")
}

/// Both real drivers raise informational cards; the Legacy wire's finalized
/// yes/no settles only the announced card and leaves its source in the chat.
/// Scenario: Something needs you during the call.
#[tokio::test]
async fn legacy_answers_a_question_once_and_records_yes_and_no_from_the_call() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (call, _, thread) = desk.call_on(&ada, None).await;
        for (seq, words, status) in [(1, "yes", "done"), (2, "no", "dismissed")] {
            let floor = desk
                .client
                .call("voice.input_state", json!({"callId":call,"speaking":true}))
                .await;
            assert_eq!(floor["ok"], true);
            let card = ask(
                &mut desk,
                &ada,
                "ask_question",
                json!({"question":"Is blue your preferred colour?"}),
            )
            .await;
            desk.advance(2).await;
            assert_eq!(
                desk.log
                    .load(&StreamId::Call(call.clone()))
                    .iter()
                    .filter(|e| e["kind"] == "agent"
                        && e["text"]
                            .as_str()
                            .is_some_and(|s| s.contains("Say yes or no")))
                    .count(),
                seq as usize - 1,
                "no new card speaks while the caller has the floor"
            );
            let floor = desk
                .client
                .call("voice.input_state", json!({"callId":call,"speaking":false}))
                .await;
            assert_eq!(floor["ok"], true);
            desk.event(thread, |event| {
                event["kind"] == "agent"
                    && event["text"]
                        .as_str()
                        .is_some_and(|s| s.contains("Say yes or no"))
            })
            .await;
            desk.say(&call, seq, words).await;
            crate::common::until("the recorded answer", || {
                desk.log.load(&StreamId::Tape(ada.clone())).iter().any(|e| {
                    e["id"] == card["id"]
                        && e["status"] == status
                        && e["note"]
                            .as_str()
                            .is_some_and(|s| s.contains(&format!("From call {call}")))
                })
            })
            .await;
            let notices: Vec<_> = desk
                .log
                .load(&StreamId::Tape(ada.clone()))
                .into_iter()
                .filter(|e| {
                    e["id"] == format!("call-answer:{call}:{}", card["actionId"].as_str().unwrap())
                })
                .collect();
            assert_eq!(notices.len(), 1);
            let text = notices[0]["text"].as_str().unwrap();
            assert!(text.starts_with(if seq == 1 {
                "Answered on the call: Yes — "
            } else {
                "Answered on the call: No — "
            }));
            assert!(!text.contains(call.as_str()));
            crate::common::until("the answer continuation ends", || {
                desk.log
                    .load(&StreamId::Tape(ada.clone()))
                    .iter()
                    .filter(|e| e["kind"] == "turn")
                    .count()
                    >= seq as usize * 2
            })
            .await;
        }
        desk.say(
            &call,
            3,
            "CALL ask_question {\"question\":\"Is the harbour north of town?\"}",
        )
        .await;
        let own_card = desk
            .event(thread, |e| {
                e["kind"] == "human_action" && e["status"] == "pending"
            })
            .await;
        desk.event(thread, |e| {
            e["kind"] == "agent"
                && e["text"].as_str().is_some_and(|s| {
                    s.contains("Is the harbour north of town?") && s.contains("Say yes or no")
                })
        })
        .await;
        desk.say(&call, 4, "yes").await;
        desk.event(thread, |e| {
            e["id"] == own_card["id"] && e["status"] == "done"
        })
        .await;
        assert!(desk.log.load(&StreamId::Tape(ada.clone())).iter().any(|e| {
            e["kind"] == "notice"
                && e["text"].as_str().is_some_and(|s| {
                    s.starts_with("Answered on the call: Yes — ")
                        && s.contains("Is the harbour north of town?")
                })
        }));
        desk.hang_up(&call, thread).await;
    }
}

/// A live provider's caller-onset event keeps the queued card silent. Only
/// a finalized caller transcript can answer it; no model tool has that reach.
/// Scenario: Something needs you during the call.
#[tokio::test]
async fn live_cards_wait_for_the_caller_and_desk_actions_refuse_spoken_yes() {
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
        let started = desk
            .client
            .call(
                "voice.call_start",
                json!({"callId":call,"personaId":ada,"mode":"front"}),
            )
            .await;
        assert_eq!(started["ok"], true, "{started}");
        crate::common::until("the front opens", || front.config.lock().unwrap().is_some()).await;
        events.send(Ok(RealtimeEvent::SpeechStarted)).await.unwrap();
        let card = ask(
            &mut desk,
            &ada,
            "ask_question",
            json!({"question":"Is blue your preferred colour?"}),
        )
        .await;
        // A controlled clock crosses the polling boundary, without an elapsed-time ceiling.
        desk.advance(2).await;
        while let Ok(command) = asked.try_recv() {
            assert!(
                !matches!(command, Command::Say(_)),
                "a card spoke over the person"
            );
        }
        events
            .send(Ok(RealtimeEvent::Heard("I'm ready.".into())))
            .await
            .unwrap();
        assert!(
            said(&mut asked)
                .await
                .contains("Is blue your preferred colour?")
        );
        events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
        events
            .send(Ok(RealtimeEvent::Heard("yes".into())))
            .await
            .unwrap();
        assert!(said(&mut asked).await.contains("recorded from this call"));
        crate::common::until("the live answer", || {
            desk.log
                .load(&StreamId::Tape(ada.clone()))
                .iter()
                .any(|e| e["id"] == card["id"] && e["status"] == "done")
        })
        .await;
        events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
        crate::common::until("the answer turn ends", || {
            desk.log
                .load(&StreamId::Tape(ada.clone()))
                .iter()
                .filter(|e| e["kind"] == "turn")
                .count()
                >= 2
        })
        .await;

        let action = ask(
            &mut desk,
            &ada,
            "request_human",
            json!({"reason":"Confirm the payment and delete the saved card on screen."}),
        )
        .await;
        assert!(
            said(&mut asked)
                .await
                .contains("cannot be answered by voice")
        );
        events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
        events
            .send(Ok(RealtimeEvent::Heard("yes".into())))
            .await
            .unwrap();
        assert!(said(&mut asked).await.contains("could not be answered"));
        let still_pending = desk
            .log
            .load(&StreamId::Tape(ada.clone()))
            .into_iter()
            .find(|e| e["id"] == action["id"])
            .unwrap();
        assert_eq!(still_pending["status"], "pending");
        let config = front.config.lock().unwrap().clone().unwrap();
        assert!(
            !config
                .tools
                .iter()
                .any(|t| t.name.contains("answer") || t.name == "ask_question")
        );
        let ended = desk
            .client
            .call("voice.call_end", json!({"callId":call}))
            .await;
        assert_eq!(ended["ok"], true);
    }
}

#[tokio::test]
async fn spoken_collaboration_approval_is_only_for_this_session() {
    let mut desk = open().await;
    let (ada, _) = desk.teammate("Ada", Driver::HotlineAgent, false).await;
    let (bob, _) = desk.teammate("Bob", Driver::HotlineAgent, false).await;
    let (call, _, thread) = desk.call_on(&ada, None).await;
    let prompted = desk.client.call("session.prompt", json!({"personaId":ada,
        "text":format!("CALL message_teammate {}", json!({"to":bob,"message":"Check the harbour","intent":"ask"}))})).await;
    assert_eq!(prompted["ok"], true);
    desk.event(thread, |e| {
        e["kind"] == "agent"
            && e["text"]
                .as_str()
                .is_some_and(|s| s.contains("allow this session"))
    })
    .await;
    desk.say(&call, 1, "yes").await;
    crate::common::until("the session approval", || {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .iter()
            .any(|e| e["kind"] == "permission" && e["decision"] == "allow_session")
    })
    .await;
    assert!(
        !desk
            .log
            .load(&StreamId::Tape(ada.clone()))
            .iter()
            .any(|e| e["decision"] == "allow_always")
    );
    assert_eq!(
        hotline_core::room::roster(&desk.log)
            .iter()
            .find(|p| p.id == bob)
            .unwrap()
            .allowed_senders,
        [] as [String; 0]
    );
    desk.hang_up(&call, thread).await;
}
