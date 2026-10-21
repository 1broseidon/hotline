use super::super::*;

#[tokio::test]
async fn calls_sixty_seconds_apart_share_one_note_and_each_source() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (first, first_thread, first_sub) = desk.call_on(&ada, None).await;
        desk.say(&first, 1, "FIRST_CALL_BEACON").await;
        desk.finished(first_sub).await;
        desk.hang_up(&first, first_sub).await;
        desk.advance(60).await;
        let (second, second_thread, second_sub) = desk.call_on(&ada, None).await;
        desk.say(&second, 1, "SECOND_CALL_HARBOUR").await;
        desk.finished(second_sub).await;
        desk.advance(61).await;
        assert!(
            desk.chat(&ada).is_empty(),
            "the first timer must be cancelled"
        );
        desk.hang_up(&second, second_sub).await;
        desk.advance(119).await;
        assert_eq!(desk.chat(&ada), [] as [Value; 0]);
        desk.advance(2).await;
        let note = desk.closing(tape).await;
        assert_eq!(
            note["from"]["sources"],
            json!([first_thread, second_thread])
        );
        for sub in [first_sub, second_sub] {
            desk.event(sub, |event| {
                event["kind"] == "link" && event["state"] == "parked"
            })
            .await;
        }
        assert_eq!(desk.chat(&ada).len(), 1);
        let requests = desk.requests.lock().unwrap();
        let summaries: Vec<_> = requests
            .iter()
            .filter(|request| request["stream"] != true)
            .collect();
        assert_eq!(summaries.len(), 1);
        let summary = summaries[0].to_string();
        assert!(summary.contains("FIRST_CALL_BEACON") && summary.contains("SECOND_CALL_HARBOUR"));
    }
}

#[tokio::test]
async fn calls_three_minutes_apart_leave_two_notes() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        for index in 0..2 {
            let (call, thread, sub) = desk.call_on(&ada, None).await;
            desk.say(&call, 1, &format!("Harbour plan {index}")).await;
            desk.finished(sub).await;
            desk.hang_up(&call, sub).await;
            desk.advance(180).await;
            let note = desk.closing(tape).await;
            assert_eq!(note["from"]["sources"], json!([thread]));
            desk.event(sub, |event| {
                event["kind"] == "link" && event["state"] == "parked"
            })
            .await;
        }
        assert_eq!(desk.chat(&ada).len(), 2);
    }
}

#[tokio::test]
async fn a_posted_work_result_and_late_reply_leave_only_the_result_and_note() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        let gate = desk.root.path().join("release-turn");
        desk.say(&call, 1, &format!("CALL post_to_chat {{\"text\":\"The sonnet is saved in qa.sonnet.txt.\"}}\nWAIT_FOR {}\nREPLY_JSON \"Done. The new sonnet is saved.\"", gate.display())).await;
        desk.event(tape, |event| event["kind"] == "agent").await;
        desk.hang_up(&call, sub).await;
        std::fs::write(gate, "finish").unwrap();
        desk.finished(sub).await;
        assert_eq!(
            desk.chat(&ada).len(),
            1,
            "the late reply adds no chat message"
        );
        desk.advance(121).await;
        let note = desk.event(tape, |event| event["kind"] == "agent").await;
        assert_eq!(
            note["text"],
            "The calls finished. The written results are already in this chat; the linked calls keep the conversation."
        );
        desk.event(sub, |event| {
            event["kind"] == "link" && event["state"] == "parked"
        })
        .await;
        assert_eq!(desk.chat(&ada).len(), 2);
        assert!(
            desk.stream(&thread)
                .iter()
                .any(|event| event["text"] == "Done. The new sonnet is saved.")
        );
        let requests = desk.requests.lock().unwrap();
        let summary = requests
            .iter()
            .rev()
            .find(|request| request["stream"] != true)
            .unwrap()
            .to_string();
        assert!(summary.contains("Already delivered") && summary.contains("qa.sonnet.txt"));
        assert!(summary.contains("never means the written result is cut off"));
    }
}
