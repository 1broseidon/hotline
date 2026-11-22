//! Call inbox results use one best-effort voice offer and an independent chat copy.

use super::*;
use crate::contract::VoiceEvent;
use crate::store::inbox as kept;
use hotline_voice::realtime::{Command, RealtimeEvent, RealtimeSession};

const TITLE: &str = "Harbour inspection";
const ASK: &str = "Bob, inspect the north pier for me.";
const COLLEAGUE: &str = "Ada, no repairs are needed at the north pier.";
const RESULT: &str = "The north pier is in good shape.";
const SENT: &str = "Bob has it.";

fn say(id: &str, words: &str) -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: id.into(),
            text: words.into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]
}

fn enrol_bob(room: &Room) {
    let mut bob = persona("bob");
    bob.name = "Bob".into();
    enrol(&room.log, &bob);
    room.allow_sender("bob", "ada").unwrap();
}

fn send_args() -> Value {
    json!({"to": ["Bob"], "title": TITLE, "message": ASK, "wants": "work"})
}

fn ask_id(room: &Room, call: &str, sent: &str) -> String {
    let sent: Value = serde_json::from_str(sent).expect("the send tool returned its receipt");
    assert_eq!(sent["sent"][0]["delivery"], "in inbox", "{sent}");
    let id = sent["sent"][0]["id"].as_str().unwrap();
    let item = kept::received(&room.log, "bob")
        .into_iter()
        .find(|item| item.id == id)
        .expect("Bob received the ask");
    assert_eq!(item.from_thread, ThreadId::call(call));
    assert_eq!(item.asked_on_call.as_deref(), Some(call));
    id.to_string()
}

async fn answer_ask(room: &Arc<Room>, id: &str) {
    TeammateTools::new(room, "bob")
        .call(
            "update_inbox",
            &json!({"items": [{"id": id, "answer": COLLEAGUE}]}),
        )
        .await
        .unwrap();
}

fn result_item(room: &Room, ask: &str) -> crate::contract::InboxItem {
    kept::received(&room.log, "ada")
        .into_iter()
        .find(|item| item.reply_to.as_deref() == Some(ask))
        .expect("Ada received the result")
}

fn chat_result(room: &Room, item: &crate::contract::InboxItem) -> Option<Value> {
    let id = format!("inbox-call-result:{}", item.id);
    tape(room, "ada")
        .into_iter()
        .find(|event| event["id"] == id)
}

async fn check_chat(room: &Room, item: &crate::contract::InboxItem) {
    until("the inbox result reached main chat", || {
        chat_result(room, item).is_some()
    })
    .await;
    let event = chat_result(room, item).unwrap();
    assert_eq!(event["kind"], "agent");
    assert_ne!(event["silent"], true);
    assert_eq!(event["from"]["kind"], "call");
    assert!(event["text"].as_str().unwrap().contains(COLLEAGUE));
}

#[tokio::test(start_paused = true)]
async fn an_answer_on_a_turn_by_turn_call_wakes_its_thread_and_speaks_once() {
    pipeline_answer(false).await;
}

#[tokio::test(start_paused = true)]
async fn an_answer_on_a_busy_call_steers_the_turn_in_flight_and_speaks_once() {
    pipeline_answer(true).await;
}

async fn pipeline_answer(busy: bool) {
    let gate = Arc::new(Semaphore::new(0));
    let result = say("result", &format!("<spoken>{RESULT}</spoken>"));
    let turns = if busy {
        vec![result]
    } else {
        vec![say("sent", &format!("<spoken>{SENT}</spoken>")), result]
    };
    let driver = Scripted::turns(turns).gated(gate.clone()).consuming();
    let name = if busy {
        "inbox-call-busy"
    } else {
        "inbox-call-idle"
    };
    let (room, fake, calls) = calling(name, driver).await;
    enrol_bob(&room);
    let call = call_ada(&room, &calls);
    let (_, mut events) = calls.subscribe(&call).unwrap();
    calls.text(&call, 1, ASK, None).unwrap();
    until("Ada is working on the call's ask", || {
        fake.prompts().iter().any(|prompt| prompt.contains(ASK))
    })
    .await;
    let tools = fake.tools().pop().expect("the call agent has its tools");
    let sent = tools.call("send", &send_args()).await.unwrap();
    let id = ask_id(&room, &call, &sent);

    if !busy {
        gate.add_permits(2);
        until("Ada is waiting for Bob", || ready(&room, &call)).await;
    }
    answer_ask(&room, &id).await;
    let item = result_item(&room, &id);
    if busy {
        until("the running call turn was told what finished", || {
            fake.steered().iter().any(|line| line.contains(TITLE))
        })
        .await;
        assert_eq!(fake.prompts().len(), 1, "no second turn was started");
    } else {
        until("the answer wakes the call's work thread", || {
            room.sides.get(&call).is_some_and(|side| side.working())
                && fake.told().iter().skip(1).any(|line| line.contains(TITLE))
        })
        .await;
    }
    gate.add_permits(2);

    let mut said = Vec::new();
    let mut audio = Vec::new();
    let mut drain = || {
        while let Ok(event) = events.try_recv() {
            match event {
                VoiceEvent::Said { id, text } => {
                    assert!(!text.contains(COLLEAGUE), "the reply was addressed to Ada");
                    if text == RESULT {
                        said.push(id);
                    }
                }
                VoiceEvent::Clip { id, data, .. } if !data.is_empty() => audio.push(id),
                _ => {}
            }
        }
        said.iter().filter(|id| audio.contains(id)).count()
    };
    until("the call sends sound for Ada's result", || drain() > 0).await;
    until("the call's work is finished", || ready(&room, &call)).await;
    // Let any separately scheduled inbox wake run before checking for a duplicate.
    tokio::time::sleep(Duration::from_secs(1)).await;
    drain();
    assert_eq!(said.len(), 1, "the result was spoken once");
    assert_eq!(fake.prompts().len(), if busy { 1 } else { 2 });
    assert!(chat_result(&room, &item).is_none());
    let mut events = room.log.subscribe(&StreamId::Tape("ada".into()));
    calls.end(&call).unwrap();
    check_chat(&room, &item).await;
    room.recover_inbox();
    room.post_ended_call_inbox("ada");
    room.inbox_due(now_ms()).await;
    let id = format!("inbox-call-result:{}", item.id);
    let mut written = 0;
    while let Ok(event) = events.try_recv() {
        if event["id"] == id {
            written += 1;
        }
    }
    assert_eq!(written, 1, "including duplicate arrival and recovery");
}

#[tokio::test(start_paused = true)]
async fn an_answer_to_the_desk_leads_front_send_wakes_its_cold_thread_and_speaks_once() {
    front_answer(false).await;
}

#[tokio::test(start_paused = true)]
async fn an_answer_after_hangup_posts_only_to_chat() {
    front_answer(true).await;
}

async fn front_answer(hung_up: bool) {
    let (session, mut asked, provider) = RealtimeSession::scripted(64);
    let front = Arc::new(Front(Mutex::new(Some(session))));
    let name = if hung_up {
        "inbox-call-ended"
    } else {
        "inbox-call-front"
    };
    let (room, fake, calls) =
        calling_with(name, Scripted::new(say("result", RESULT)), Some(front)).await;
    enrol_bob(&room);
    room.log
        .append(
            &StreamId::Room,
            &crate::room::room_event("setting", json!({"id": "teamLead", "value": "ada"})),
        )
        .unwrap();
    let call = uuid::Uuid::new_v4().to_string();
    calls
        .start_call(
            &call,
            CallRequest {
                target: None,
                thread: None,
                stream_audio: true,
                input_mode: VoiceInputMode::Audio,
                mode: VoiceCallMode::Front,
                mic_rate: Some(24_000),
            },
            handle(&room),
        )
        .unwrap();
    provider
        .send(Ok(RealtimeEvent::Heard(ASK.into())))
        .await
        .unwrap();
    provider
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "send-to-bob".into(),
            name: "send".into(),
            arguments: send_args().to_string(),
        }))
        .await
        .unwrap();
    let mut sent = None;
    until("the desk front got the send receipt", || {
        while let Ok(command) = asked.try_recv() {
            if let Command::ToolResult {
                call_id, output, ..
            } = command
                && call_id == "send-to-bob"
            {
                sent = Some(output);
                return true;
            }
        }
        false
    })
    .await;
    let id = ask_id(&room, &call, &sent.unwrap());
    assert!(room.sides.get(&call).is_none(), "no work agent is up yet");
    assert_eq!(fake.prompts(), [] as [String; 0]);
    if hung_up {
        calls.end(&call).unwrap();
    }
    answer_ask(&room, &id).await;

    let item = result_item(&room, &id);
    if hung_up {
        check_chat(&room, &item).await;
        assert_eq!(fake.prompts(), [] as [String; 0]);
        assert!(room.sides.get(&call).is_none());
        assert!(
            !std::iter::from_fn(|| asked.try_recv().ok()).any(|cmd| matches!(cmd, Command::Say(_)))
        );
    } else {
        let mut spoken = Vec::new();
        until(
            "the cold call's work thread gives the front the result",
            || {
                while let Ok(command) = asked.try_recv() {
                    if let Command::Say(text) = command {
                        spoken.push(text);
                    }
                }
                spoken.iter().any(|text| text == RESULT)
            },
        )
        .await;
        assert_eq!(spoken, [RESULT]);
        assert_eq!(fake.prompts().len(), 1);
        assert!(fake.told()[0].contains(COLLEAGUE));
        assert!(chat_result(&room, &item).is_none());
        calls.end(&call).unwrap();
        check_chat(&room, &item).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_busy_call_does_not_replay_unconsumed_inbox_steering() {
    let gate = Arc::new(Semaphore::new(0));
    let driver = Scripted::new(say("sent", SENT))
        .gated(gate.clone())
        .steerable();
    let (room, fake, calls) = calling("inbox-call-unconsumed", driver).await;
    enrol_bob(&room);
    let call = call_ada(&room, &calls);
    calls.text(&call, 1, ASK, None).unwrap();
    until("Ada is working on the call's ask", || {
        !fake.prompts().is_empty()
    })
    .await;
    let tools = fake.tools().pop().unwrap();
    let sent = tools.call("send", &send_args()).await.unwrap();
    let id = ask_id(&room, &call, &sent);
    answer_ask(&room, &id).await;
    until("the busy turn received steering it won't consume", || {
        fake.steered().iter().any(|line| line.contains(COLLEAGUE))
    })
    .await;
    gate.add_permits(2);
    until("the thread is idle without replay", || ready(&room, &call)).await;
    assert_eq!(fake.prompts().len(), 1);
    calls.end(&call).unwrap();
    check_chat(&room, &result_item(&room, &id)).await;
}

#[tokio::test(start_paused = true)]
async fn startup_keeps_an_old_unposted_call_result_until_it_reaches_chat() {
    let fake = Fake::new(Scripted::new(say("unused", RESULT)));
    let room = room("inbox-call-recovery", fake.clone());
    let call = uuid::Uuid::new_v4().to_string();
    // A crash after the item was kept and the live offer claimed, before a
    // chat copy. Use an old closed item to exercise startup's earlier tidy.
    let item: crate::contract::InboxItem = serde_json::from_value(json!({
        "id": "saved-result", "ts": 1, "from": "bob", "to": "ada", "title": TITLE,
        "body": COLLEAGUE, "state": "closed", "wants": "nothing", "senderTold": 1,
        "fromThread": ThreadId::dm("bob"), "about": ThreadId::call(&call),
        "replyTo": "saved-ask", "askedOnCall": call, "offered": 2
    }))
    .unwrap();
    room.keep_item(&item).unwrap();
    room.inbox_tidy(now_ms(), true);
    assert!(chat_result(&room, &item).is_none());
    room.recover_inbox();
    check_chat(&room, &item).await;
    assert_eq!(fake.prompts(), [] as [String; 0]);
    room.inbox_tidy(now_ms(), true);
    assert_eq!(kept::received(&room.log, "ada"), []);
}
