//! The same scripted provider ordering is exercised in both socket dialects.
use super::*;

#[tokio::test]
async fn a_barged_question_commits_before_any_client_reply_can_overtake_it() {
    for (gpt, early_done) in [(false, true), (false, false), (true, true), (true, false)] {
        let (listener, url) = listen().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = accept(&listener).await;
            handshake(&mut socket).await;
            send(&mut socket, json!({"type":"response.created"})).await;
            send(
                &mut socket,
                json!({"type":"response.output_audio.delta",
                "item_id":"rundown", "delta":pcm(&[1, 0])}),
            )
            .await;
            send(
                &mut socket,
                json!({"type":"input_audio_buffer.speech_started",
                "item_id":"question"}),
            )
            .await;
            // VAD already cancels generation. A second cancel can race that
            // cancellation; the only playback command needed is truncation.
            let cut = receive(&mut socket).await;
            assert_eq!(cut["type"], "conversation.item.truncate");
            assert_eq!(cut["item_id"], "rundown");
            send(&mut socket, json!({"type":"conversation.item.truncated"})).await;
            if early_done {
                send(
                    &mut socket,
                    json!({"type":"response.done",
                "response":{"status":"cancelled"}}),
                )
                .await;
            }
            // A completed tool arrives while the interrupting question is
            // still being spoken. Keep its output, but do not create a reply.
            assert_eq!(
                receive(&mut socket).await["item"]["type"],
                "function_call_output"
            );
            assert_eq!(receive(&mut socket).await["item"]["type"], "message");
            assert_eq!(
                receive(&mut socket).await["type"],
                "input_audio_buffer.append"
            );
            if !early_done {
                send(
                    &mut socket,
                    json!({"type":"response.done",
                "response":{"status":"cancelled"}}),
                )
                .await;
            }
            send(
                &mut socket,
                json!({"type":"input_audio_buffer.speech_stopped",
                "item_id":"question"}),
            )
            .await;
            send(
                &mut socket,
                json!({"type":"input_audio_buffer.committed",
                "item_id":"question"}),
            )
            .await;
            send(
                &mut socket,
                json!({"type":"conversation.item.input_audio_transcription.completed",
                "item_id":"question", "transcript":"Wait. Which one was the newest?"}),
            )
            .await;
            // A barrier after commit proves we did not race the automatic
            // response with a manual create, clear, or second commit.
            assert_eq!(
                receive(&mut socket).await["item"]["type"],
                "function_call_output"
            );
            assert_eq!(
                receive(&mut socket).await["type"],
                "input_audio_buffer.append"
            );
            send(&mut socket, json!({"type":"response.created"})).await;
            send(
                &mut socket,
                json!({"type":"response.output_audio_transcript.delta",
                "delta":"The call notes were newest."}),
            )
            .await;
            send(
                &mut socket,
                json!({"type":"response.output_audio.delta",
                "item_id":"answer", "delta":pcm(&[2, 0])}),
            )
            .await;
            send(&mut socket, json!({"type":"response.done"})).await;
            // The automatic answer includes the queued context. No stale
            // continuation follows it, nor a stale tool_choice=none.
            assert_eq!(
                receive(&mut socket).await["item"]["type"],
                "function_call_output"
            );
            let reply = receive(&mut socket).await;
            assert_eq!(reply["type"], "response.create");
            assert!(reply["response"]["tool_choice"].is_null());
            closed(&mut socket).await;
        });
        let provider = if gpt { openai(&url) } else { voice(&url) };
        let mut configuration = config();
        configuration.input_rate = 24_000;
        let mut session = provider.open(configuration).await.unwrap();
        assert!(matches!(next(&mut session).await, RealtimeEvent::Audio(_)));
        assert_eq!(next(&mut session).await, RealtimeEvent::SpeechStarted);
        session.input.truncate(26_000).await.unwrap();
        if early_done {
            assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
        }
        session
            .input
            .tool_result("work", "finished", true)
            .await
            .unwrap();
        session.input.say("The checks passed.").await.unwrap();
        session.input.push_audio(vec![0, 0]).await.unwrap();
        if !early_done {
            assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
        }
        assert_eq!(
            next(&mut session).await,
            RealtimeEvent::Heard("Wait. Which one was the newest?".into())
        );
        session
            .input
            .tool_result("after-commit", "also finished", true)
            .await
            .unwrap();
        session.input.push_audio(vec![0, 0]).await.unwrap();
        assert_eq!(
            next(&mut session).await,
            RealtimeEvent::Saying("The call notes were newest.".into())
        );
        assert_eq!(next(&mut session).await, RealtimeEvent::Audio(vec![2, 0]));
        assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
        session
            .input
            .tool_result("next", "done", true)
            .await
            .unwrap();
        session.close().await;
        finish(server).await;
    }
}

/// Grok creates its automatic reply before it commits the caller's audio. That
/// turn is then over: a work result arriving later is still answered.
#[tokio::test]
async fn a_reply_created_before_the_commit_ends_the_caller_turn() {
    for gpt in [false, true] {
        let (listener, url) = listen().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = accept(&listener).await;
            handshake(&mut socket).await;
            for event in [
                json!({"type":"input_audio_buffer.speech_started", "item_id":"ask"}),
                json!({"type":"input_audio_buffer.speech_stopped", "item_id":"ask"}),
                json!({"type":"response.created"}),
                json!({"type":"input_audio_buffer.committed", "item_id":"ask"}),
                json!({"type":"conversation.item.input_audio_transcription.completed",
                    "item_id":"ask", "transcript":"Write the sonnet to a file."}),
                json!({"type":"response.done"}),
            ] {
                send(&mut socket, event).await;
            }
            assert_eq!(
                receive(&mut socket).await["item"]["type"],
                "function_call_output"
            );
            assert_eq!(receive(&mut socket).await["type"], "response.create");
            closed(&mut socket).await;
        });
        let provider = if gpt { openai(&url) } else { voice(&url) };
        let mut configuration = config();
        configuration.input_rate = 24_000;
        let mut session = provider.open(configuration).await.unwrap();
        assert_eq!(next(&mut session).await, RealtimeEvent::SpeechStarted);
        assert_eq!(
            next(&mut session).await,
            RealtimeEvent::Heard("Write the sonnet to a file.".into())
        );
        assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
        session
            .input
            .tool_result("work", "Saved qa.sonnet.txt.", true)
            .await
            .unwrap();
        session.close().await;
        finish(server).await;
    }
}

/// A line given to say is spoken once and its "Say exactly this…" instruction
/// cannot be read out by a later reply (George's call, 13:11).
#[tokio::test]
async fn a_spoken_line_leaves_no_instruction_behind() {
    for gpt in [false, true] {
        let (listener, url) = listen().await;
        let server = tokio::spawn(async move {
            let (mut socket, _) = accept(&listener).await;
            handshake(&mut socket).await;
            let item = receive(&mut socket).await;
            assert_eq!(item["type"], "conversation.item.create");
            // The provider names the item, as Grok does, whatever the client sent.
            let mut created = item["item"].clone();
            created["id"] = json!("item_provider_7");
            send(
                &mut socket,
                json!({"type":"conversation.item.created", "item": created}),
            )
            .await;
            let id = "item_provider_7";
            assert_eq!(receive(&mut socket).await["type"], "response.create");
            send(&mut socket, json!({"type":"response.created"})).await;
            send(&mut socket, json!({"type":"response.done"})).await;
            // OpenAI removes the instruction once spoken. Grok says lines only
            // from user items and cannot delete them; its rules forbid quoting.
            assert_eq!(item["item"]["role"], if gpt { "system" } else { "user" });
            if gpt {
                let deleted = receive(&mut socket).await;
                assert_eq!(deleted["type"], "conversation.item.delete");
                assert_eq!(deleted["item_id"], id);
            }
            closed(&mut socket).await;
        });
        let provider = if gpt { openai(&url) } else { voice(&url) };
        let mut configuration = config();
        configuration.input_rate = 24_000;
        let mut session = provider.open(configuration).await.unwrap();
        session
            .input
            .say("A question for you: ready? Say yes or no.")
            .await
            .unwrap();
        assert_eq!(next(&mut session).await, RealtimeEvent::ResponseDone);
        session.close().await;
        finish(server).await;
    }
}
