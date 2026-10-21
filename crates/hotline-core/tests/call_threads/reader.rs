use super::super::*;

/// Scenario: Following up in the chat (task-206). The call alone knows the detail.
#[tokio::test]
async fn the_home_agent_reads_the_call_transcript_to_recover_a_detail_only_said_there() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "The mascot is Bramble, a wiry terrier. Use that exact description.\nCALL post_to_chat {\"text\":\"The mascot plan is ready.\"}").await;
        desk.finished(sub).await;
        desk.hang_up(&call, sub).await;
        assert!(
            !desk
                .chat(&ada)
                .iter()
                .any(|event| event.to_string().contains("Bramble"))
        );
        let prompt = "Like we said on the call, use the mascot exactly that way. What is its name and description?";
        let sent = desk
            .client
            .call("session.prompt", json!({"personaId":ada,"text":prompt}))
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
        let reply = desk
            .event(tape, |event| {
                event["kind"] == "agent"
                    && event["from"].is_null()
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("Bramble"))
            })
            .await;
        let text = reply["text"].as_str().unwrap();
        assert!(text.contains("Bramble, a wiry terrier"));
        assert!(text.contains("read_thread"));
        assert!(!text.contains("search_thread"));
        assert!(text.contains("IS the call's transcript"));
        assert!(text.contains(thread["key"].as_str().unwrap()));
        // The deterministic drivers can only answer from the actual tool
        // response; neither fixture contains the mascot detail.
    }
}

#[tokio::test]
async fn thread_reads_page_in_order_and_refuse_another_teammates_thread_on_both_drivers() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (bea, _) = desk.teammate("Bea", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "Bramble, a wiry terrier.\nCALL list_chapters {}")
            .await;
        desk.finished(sub).await;
        desk.hang_up(&call, sub).await;
        let events = desk.stream(&thread);
        let key = format!("call:{call}");
        let first = &events[0]["id"];
        let text = format!(
            "CALL read_thread {}\nCALL read_thread {}\nCALL read_thread {}\nCALL read_thread {}\nCALL search_thread {{\"query\":\"Bramble\"}}",
            json!({"thread":key,"limit":1}),
            json!({"thread":key,"after":first,"limit":100}),
            json!({"thread":format!("dm:{bea}")}),
            json!({"thread":key,"after":"missing"})
        );
        desk.client
            .call("session.prompt", json!({"personaId":ada,"text":text}))
            .await;
        let reply = desk
            .event(tape, |event| {
                event["kind"] == "agent"
                    && event["from"].is_null()
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("RESULTS"))
            })
            .await;
        let text = reply["text"].as_str().unwrap();
        assert!(text.contains(&format!("\"next_after\":{first}")), "{text}");
        assert!(text.contains("\"next_after\":null"), "{text}");
        assert!(text.contains("Bramble, a wiry terrier"), "{text}");
        assert!(text.contains("No such thread of yours"), "{text}");
        assert!(text.contains("That cursor is not in this thread"), "{text}");
        assert!(
            text.contains("This call's thread IS its transcript"),
            "{text}"
        );
        // The real call record contains the handler's tool line in its own
        // position, including the in-process driver's completed update.
        for event in events.iter().filter(|event| event["kind"] == "tool") {
            assert!(text.contains(event["id"].as_str().unwrap()));
        }
    }
}

#[tokio::test]
async fn work_and_job_threads_are_read_by_the_keys_the_home_agent_receives() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let work = desk
            .client
            .call(
                "thread.open",
                json!({"personaId":ada,"text":"Work-only detail: the collar is blue."}),
            )
            .await;
        assert_eq!(work["ok"], true, "{work}");
        let thread = work["result"]["thread"].clone();
        let work_sub = desk.client.subscribe(json!({"threadId":thread})).await;
        desk.finished(work_sub).await;
        let job = desk.client.call("schedule.create", json!({"personaId":ada,"kind":"schedule","when":chrono::Utc::now().timestamp_millis()+1200,"prompt":"Job-only detail: the tag is brass.","quiet":true})).await;
        assert_eq!(job["ok"], true, "{job}");
        let job_id = job["result"]["id"].as_str().unwrap();
        let job_sub = desk
            .client
            .subscribe(json!({"threadId":{"kind":"job","key":job_id}}))
            .await;
        desk.finished(job_sub).await;
        let prompt = format!(
            "CALL read_thread {}\nCALL read_thread {}",
            json!({"thread":format!("side:{}",thread["key"].as_str().unwrap())}),
            json!({"thread":format!("job:{job_id}")})
        );
        desk.client
            .call("session.prompt", json!({"personaId":ada,"text":prompt}))
            .await;
        let reply = desk
            .event(tape, |event| {
                event["kind"] == "agent"
                    && event["from"].is_null()
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("RESULTS"))
            })
            .await;
        let text = reply["text"].as_str().unwrap();
        assert!(text.contains("the collar is blue"), "{text}");
        assert!(text.contains("the tag is brass"), "{text}");
    }
}
