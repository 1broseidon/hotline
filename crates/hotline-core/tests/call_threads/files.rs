use super::super::*;
use super::live_work::{Front, event, result};
use hotline_voice::realtime::{RealtimeEvent, RealtimeSession};
use std::fmt::Write as _;
use std::sync::Mutex;

/// Real wire, work admission, both drivers, filesystem tools and stored-file
/// readback. The realtime provider is scripted; no model prose supplies paths.
#[tokio::test]
async fn call_files_are_delivered_once_and_large_batches_are_one_openable_card() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        for (count, send) in [
            (0, false),
            (1, false),
            (1, true),
            (3, false),
            (6, false),
            (6, true),
        ] {
            let (session, mut asked, events) = RealtimeSession::scripted(64);
            let front = Arc::new(Front {
                provider: "grok",
                session: Mutex::new(Some(session)),
                config: Mutex::new(None),
            });
            let mut desk = open_with_front(Some(front.clone())).await;
            let (ada, _) = desk.teammate("Ada", driver, false).await;
            // Merely present files are never evidence of work on this call.
            std::fs::write(
                std::path::Path::new(&desk.workspace).join("unrelated.txt"),
                "private",
            )
            .unwrap();
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
            let mut words = String::from("Write the requested files.\n");
            if count == 0 {
                writeln!(
                    words,
                    "CALL write {}",
                    json!({"path":desk.root.path().join("outside.txt"),"content":"must be refused"})
                )
                .unwrap();
            }

            for i in 0..count {
                let dir = std::path::Path::new(&desk.workspace).join(i.to_string());
                std::fs::create_dir_all(&dir).unwrap();
                // Equal basenames must keep distinct bytes and readback positions.
                let path = dir.join("note.txt");
                if i % 2 == 1 {
                    std::fs::write(&path, "old").unwrap();
                }
                let path = if matches!(driver, Driver::HotlineAgent) {
                    std::path::PathBuf::from(format!("{i}/note.txt"))
                } else {
                    path
                };
                let text = format!("result {i}");
                let (tool, args) = if i % 2 == 1 {
                    (
                        "edit",
                        json!({"path":path,"old_text":"old","new_text":text}),
                    )
                } else {
                    ("write", json!({"path":path,"content":text}))
                };
                writeln!(words, "CALL {tool} {args}").unwrap();
                if send {
                    writeln!(words,
                        "CALL send_file {}",
                        json!({"source":"workspace","path":format!("{i}/note.txt"),"caption":"Finished file"})
                    ).unwrap();
                }
            }
            words.push_str("REPLY_JSON \"Done.\"");
            event(
                &events,
                RealtimeEvent::Heard("Write the requested files.".into()),
            )
            .await;
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
            let posts: Vec<_> = desk
                .chat(&ada)
                .into_iter()
                .filter(|line| line["attachments"].is_array())
                .collect();
            assert_eq!(
                posts.len(),
                if count > 3 { 1 } else { count },
                "count={count} send={send}: {posts:?}; call={:?}",
                desk.log.load(&stream)
            );
            let mut seen = 0;
            for post in &posts {
                assert_eq!(post["from"]["kind"], "call");
                assert_eq!(post["from"]["thread"], call);
                for (index, file) in post["attachments"].as_array().unwrap().iter().enumerate() {
                    assert_eq!(file["name"], "note.txt");
                    assert_eq!(file["created"], seen % 2 == 0);
                    assert_eq!(file["size"], 8);
                    let read = desk
                        .client
                        .call(
                            "file.read",
                            json!({"personaId":ada,"eventId":post["id"],"index":index}),
                        )
                        .await;
                    assert_eq!(read["ok"], true, "{read}");
                    use base64::Engine;
                    let bytes = base64::prelude::BASE64_STANDARD
                        .decode(read["result"]["data"].as_str().unwrap())
                        .unwrap();
                    assert_eq!(bytes, format!("result {seen}").as_bytes());
                    seen += 1;
                }
            }
            assert_eq!(seen, count);
            desk.client
                .call("voice.call_end", json!({"callId":call}))
                .await;
        }
    }
}
