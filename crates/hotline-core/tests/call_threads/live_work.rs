use super::super::*;
use hotline_voice::realtime::{Command, Realtime, RealtimeConfig, RealtimeEvent, RealtimeSession};
use hotline_voice::speech::{SpeechError, SpeechId};
use std::sync::Mutex;
use tokio::sync::mpsc;

pub(super) struct Front {
    pub(super) provider: &'static str,
    pub(super) session: Mutex<Option<RealtimeSession>>,
    pub(super) config: Mutex<Option<RealtimeConfig>>,
}
#[async_trait::async_trait]
impl Realtime for Front {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: self.provider.into(),
            model_id: "voice".into(),
            voice: None,
        }
    }
    fn is_subscription(&self) -> bool {
        true
    }
    fn delegates(&self) -> bool {
        self.provider == "gpt-live"
    }
    async fn open(&self, config: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
        *self.config.lock().unwrap() = Some(config);
        self.session
            .lock()
            .unwrap()
            .take()
            .ok_or(SpeechError::Cancelled)
    }
}
async fn result(asked: &mut mpsc::Receiver<Command>, id: &str) -> String {
    tokio::time::timeout(PATIENCE, async {
        loop {
            if let Some(Command::ToolResult {
                call_id, output, ..
            }) = asked.recv().await
                && call_id == id
            {
                return output;
            }
        }
    })
    .await
    .expect("work tool answers")
}
async fn event(events: &mpsc::Sender<Result<RealtimeEvent, SpeechError>>, event: RealtimeEvent) {
    events.send(Ok(event)).await.unwrap();
}

/// Every live front uses the same work admission, with both real drivers.
/// The fixture writes through the actual file/post handlers, then read_thread
/// reads the speech and tool lines in their folded order.
#[tokio::test]
async fn live_work_uses_its_own_driver_and_posts_its_file_without_touching_home() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        for provider in ["grok", "openai", "gpt-live"] {
            let (session, mut asked, events) = RealtimeSession::scripted(64);
            let front = Arc::new(Front {
                provider,
                session: Mutex::new(Some(session)),
                config: Mutex::new(None),
            });
            let mut desk = open_with_front(Some(front.clone())).await;
            let (ada, tape) = desk.teammate("Ada", driver, false).await;
            let path = std::path::Path::new(&desk.workspace).join("harbour-plan.txt");
            std::fs::write(&path, "Harbour beacon: use a blue lens.").unwrap();
            let call = uuid::Uuid::new_v4().to_string();
            let started = desk
                .client
                .call(
                    "voice.call_start",
                    json!({"callId":call,"personaId":ada,"mode":"front"}),
                )
                .await;
            assert_eq!(started["ok"], true, "{started}");
            crate::common::until("live front opens", || {
                front.config.lock().unwrap().is_some()
            })
            .await;
            let before = desk.log.load(&StreamId::Tape(ada.clone()));
            let config = front.config.lock().unwrap().clone().unwrap();
            assert!(config.tools.iter().any(|tool| tool.name == "work_on_it"));
            assert!(
                !config.tools.iter().any(
                    |tool| ["send_file", "post_to_chat", "shell"].contains(&tool.name.as_str())
                )
            );
            event(
                &events,
                RealtimeEvent::Heard("The harbour beacon needs a blue lens.".into()),
            )
            .await;
            event(&events, RealtimeEvent::Saying("A blue lens for".into())).await;
            event(
                &events,
                RealtimeEvent::Saying(" the harbour beacon.".into()),
            )
            .await;
            event(&events, RealtimeEvent::ResponseDone).await;
            event(
                &events,
                RealtimeEvent::Saying("The draft could include".into()),
            )
            .await;
            event(&events, RealtimeEvent::SpeechStarted).await;
            event(&events, RealtimeEvent::Heard("Write that down.".into())).await;
            event(
                &events,
                RealtimeEvent::Saying("I'll write the beacon plan.".into()),
            )
            .await;
            event(&events, RealtimeEvent::ResponseDone).await;
            let words = format!("Write that down.\nCALL send_file {}\nREPLY_JSON {}", json!({"source":"workspace","path":"harbour-plan.txt","caption":"Harbour beacon plan: the blue lens specification."}), serde_json::to_string("<spoken>The harbour beacon plan is in the chat.</spoken><written>The harbour beacon plan specifies a blue lens.</written>").unwrap());
            event(
                &events,
                RealtimeEvent::ToolCall {
                    call_id: "work".into(),
                    name: "work_on_it".into(),
                    arguments: json!({"words":words}).to_string(),
                },
            )
            .await;
            assert!(result(&mut asked, "work").await.contains("Work accepted"));
            let stream = StreamId::Call(call.clone());
            crate::common::until("call work completes", || {
                desk.log
                    .load(&stream)
                    .iter()
                    .any(|line| line["title"] == "Work result")
            })
            .await;
            // Only the final result is offered to the realtime provider to say.
            let spoken = tokio::time::timeout(PATIENCE, async {
                loop {
                    if let Some(Command::Say(text)) = asked.recv().await {
                        break text;
                    }
                }
            })
            .await
            .expect("finished result reaches live voice");
            assert!(
                spoken.contains("The harbour beacon plan is in the chat."),
                "{spoken}"
            );
            assert!(!spoken.contains("Saving the beacon plan"));
            event(
                &events,
                RealtimeEvent::Saying("The harbour beacon plan is ".into()),
            )
            .await;
            event(&events, RealtimeEvent::Saying("in the chat.".into())).await;
            event(&events, RealtimeEvent::ResponseDone).await;
            let after = desk.log.load(&StreamId::Tape(ada.clone()));
            assert_eq!(
                after.iter().filter(|line| line["kind"] == "user").count(),
                before.iter().filter(|line| line["kind"] == "user").count()
            );
            let posts: Vec<_> = after
                .iter()
                .filter(|line| line["from"]["thread"] == call && line["kind"] == "agent")
                .collect();
            assert_eq!(posts.len(), 1, "{posts:?}");
            assert!(posts[0].to_string().contains("Harbour beacon plan"));
            assert!(
                posts[0].to_string().contains("harbour-plan.txt"),
                "{posts:?}"
            );
            crate::common::until("voice result is recorded", || {
                desk.log
                    .load(&stream)
                    .iter()
                    .any(|line| line["text"] == "The harbour beacon plan is in the chat.")
            })
            .await;
            let lines = desk.log.load(&stream);
            let results: Vec<_> = lines
                .iter()
                .filter(|line| line["title"] == "Work result")
                .collect();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["kind"], "tool");
            assert!(
                results[0]["output"]
                    .to_string()
                    .contains("specifies a blue lens")
            );
            let speech: Vec<_> = lines
                .iter()
                .filter(|line| line["kind"] == "agent")
                .collect();
            assert_eq!(
                speech.len(),
                4,
                "only the four actual voice lines: {speech:?}"
            );
            assert!(
                speech
                    .iter()
                    .all(|line| line["id"].as_str().unwrap().starts_with("front-"))
            );
            let raw = std::fs::read_to_string(
                desk.root.path().join("calls").join(format!("{call}.jsonl")),
            )
            .unwrap();
            let raw: Vec<Value> = raw
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                raw.iter()
                    .filter(|line| line["text"] == "A blue lens for the harbour beacon.")
                    .count(),
                1
            );
            assert!(!raw.iter().any(|line| line["text"] == "A blue lens for"));
            assert!(
                lines
                    .iter()
                    .any(|line| line["text"] == "The draft could include"),
                "interrupted speech stays in the transcript"
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line["text"] == "I'll write the beacon plan.")
            );
            let heard = lines
                .iter()
                .position(|line| line["text"] == "The harbour beacon needs a blue lens.")
                .unwrap();
            let said = lines
                .iter()
                .position(|line| line["text"] == "A blue lens for the harbour beacon.")
                .unwrap();
            let work = lines
                .iter()
                .position(|line| line["title"] == "work_on_it")
                .unwrap();
            assert!(heard < said && said < work, "{lines:?}");
            for line in lines.iter().filter(|line| line["kind"] == "tool") {
                serde_json::from_value::<hotline_core::contract::TranscriptEvent>(line.clone())
                    .unwrap();
            }
            // Page through the real reader, within the live tool's output bound.
            let mut read_lines = Vec::new();
            let mut cursor = None;
            while read_lines.len() < lines.len() {
                let mut args = json!({"thread":format!("call:{call}"), "limit":2});
                if let Some(after) = cursor {
                    args["after"] = after;
                }
                event(
                    &events,
                    RealtimeEvent::ToolCall {
                        call_id: "read".into(),
                        name: "read_thread".into(),
                        arguments: args.to_string(),
                    },
                )
                .await;
                let read = result(&mut asked, "read").await;
                let page: Value =
                    serde_json::from_str(&read[read.find('{').unwrap()..=read.rfind('}').unwrap()])
                        .expect(&read);
                let page_lines = page["lines"].as_array().unwrap();
                assert_ne!(page_lines.len(), 0);
                cursor = Some(page_lines.last().unwrap()["id"].clone());
                read_lines.extend(page_lines.clone());
            }
            assert_eq!(
                read_lines
                    .iter()
                    .filter(|line| line["kind"] == "agent")
                    .count(),
                4
            );
            assert_eq!(
                read_lines
                    .iter()
                    .filter(|line| line["title"] == "Work result")
                    .count(),
                1
            );
            assert!(
                desk.log
                    .load(&stream)
                    .iter()
                    .any(|line| line["title"] == "read_thread")
            );
            if matches!(driver, Driver::HotlineAgent) {
                let requests = desk.requests.lock().unwrap();
                let work = requests
                    .iter()
                    .find(|request| request.to_string().contains("The whole call so far"))
                    .unwrap();
                assert_eq!(work["model"], "vendor/coder");
                assert!(
                    work.to_string()
                        .contains("A blue lens for the harbour beacon.")
                );
            }
            desk.client
                .call("voice.call_end", json!({"callId":call}))
                .await;
            desk.advance(31).await;
            desk.closing(tape).await;
            let requests = desk.requests.lock().unwrap();
            assert!(
                requests
                    .iter()
                    .filter(|request| request["stream"] != true)
                    .any(|request| {
                        let text = request.to_string();
                        text.contains("Work result") && text.contains("specifies a blue lens")
                    }),
                "closing summary includes the work even though it was never speech"
            );
        }
    }
}

#[tokio::test]
async fn changing_and_dropping_live_work_keeps_one_call_agent_and_home_keeps_running() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let (session, mut asked, events) = RealtimeSession::scripted(64);
        let front = Arc::new(Front {
            provider: "grok",
            session: Mutex::new(Some(session)),
            config: Mutex::new(None),
        });
        let mut desk = open_with_front(Some(front.clone())).await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let home_gate = desk.root.path().join("home-finish");
        let home = format!(
            "CALL request_human {{\"reason\":\"Wait for the harbour plan\"}}\nWAIT_FOR {}\nREPLY_JSON \"Home kept working.\"",
            home_gate.display()
        );
        desk.client
            .call("session.prompt", json!({"personaId":ada,"text":home}))
            .await;
        let tape = StreamId::Tape(ada.clone());
        crate::common::until("home starts its independent work", || {
            desk.log
                .load(&tape)
                .iter()
                .any(|line| line["kind"] == "human_action")
        })
        .await;
        let call = uuid::Uuid::new_v4().to_string();
        let started = desk
            .client
            .call(
                "voice.call_start",
                json!({"callId":call,"personaId":ada,"mode":"front"}),
            )
            .await;
        assert_eq!(started["ok"], true, "{started}");
        crate::common::until("front opens", || front.config.lock().unwrap().is_some()).await;
        let gate = desk.root.path().join("call-finish");
        let words = format!(
            "Draft the harbour plan.\nCALL request_human {{\"reason\":\"Wait for the harbour plan\"}}\nWAIT_FOR {}",
            gate.display()
        );
        event(&events, RealtimeEvent::Heard(words.clone())).await;
        event(
            &events,
            RealtimeEvent::ToolCall {
                call_id: "start".into(),
                name: "work_on_it".into(),
                arguments: json!({"words":words}).to_string(),
            },
        )
        .await;
        assert!(result(&mut asked, "start").await.contains("Work accepted"));
        let stream = StreamId::Call(call.clone());
        crate::common::until("call agent runs its tool", || {
            desk.log
                .load(&stream)
                .iter()
                .any(|line| line["kind"] == "human_action")
        })
        .await;
        event(
            &events,
            RealtimeEvent::Heard("Use a red lens instead.".into()),
        )
        .await;
        event(
            &events,
            RealtimeEvent::ToolCall {
                call_id: "change".into(),
                name: "work_on_it".into(),
                arguments: json!({"words":"Use a red lens instead."}).to_string(),
            },
        )
        .await;
        assert!(result(&mut asked, "change").await.contains("Work accepted"));
        // Speech and reads continue before either model is allowed to finish.
        event(&events, RealtimeEvent::Heard("Are you still there?".into())).await;
        event(
            &events,
            RealtimeEvent::Saying("I'm here; the plan will use red.".into()),
        )
        .await;
        event(&events, RealtimeEvent::ResponseDone).await;
        event(
            &events,
            RealtimeEvent::ToolCall {
                call_id: "lookup".into(),
                name: "read_thread".into(),
                arguments: json!({"thread":format!("call:{call}")}).to_string(),
            },
        )
        .await;
        assert!(result(&mut asked, "lookup").await.contains("red lens"));
        let listed = desk
            .client
            .call("thread.list", json!({"personaId":ada}))
            .await;
        assert_eq!(listed["ok"], true, "{listed}");
        assert_eq!(
            listed["result"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["thread"]["kind"] == "call")
                .count(),
            1
        );
        event(
            &events,
            RealtimeEvent::ToolCall {
                call_id: "stop".into(),
                name: "work_on_it".into(),
                arguments: json!({"words":"Never mind, drop it.","action":"stop"}).to_string(),
            },
        )
        .await;
        let stopped = result(&mut asked, "stop").await;
        assert!(
            stopped.contains("Stopped work") && stopped.contains("unfinished"),
            "{stopped}"
        );
        std::fs::write(&gate, "finish").unwrap();
        std::fs::write(&home_gate, "finish").unwrap();
        crate::common::until("home finishes without interruption", || {
            desk.log
                .load(&tape)
                .iter()
                .any(|line| line["text"] == "Home kept working.")
        })
        .await;
        crate::common::until("call cancellation settles", || {
            desk.log
                .load(&stream)
                .iter()
                .any(|line| line["kind"] == "turn")
        })
        .await;
        tokio::time::timeout(PATIENCE, async {
            loop {
                let listed = desk
                    .client
                    .call("thread.list", json!({"personaId":ada}))
                    .await;
                let row = listed["result"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["thread"]["key"] == call)
                    .unwrap();
                if row["working"] == false {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("cancelled work stays idle");
        assert_eq!(
            desk.log
                .load(&stream)
                .iter()
                .filter(|line| line["kind"] == "turn")
                .count(),
            1,
            "a dropped correction never starts another turn"
        );
        let home_users = desk
            .log
            .load(&tape)
            .into_iter()
            .filter(|line| line["kind"] == "user")
            .count();
        assert_eq!(home_users, 1, "call work did not steer the home agent");
        assert!(
            desk.log
                .load(&stream)
                .iter()
                .any(|line| line["text"] == "Work: Use a red lens instead.")
        );
        desk.client
            .call("voice.call_end", json!({"callId":call}))
            .await;
    }
}
