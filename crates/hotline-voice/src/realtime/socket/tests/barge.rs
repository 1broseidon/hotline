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
