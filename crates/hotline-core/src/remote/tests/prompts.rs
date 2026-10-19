use super::*;

#[tokio::test]
async fn an_uncertain_prompt_is_never_replayed_and_operation_ids_cannot_change_meaning() {
    let h = Harness::new().await;
    let grant = h.pair().await;
    let phone = h.remote.authenticate_v2(&grant.public).unwrap();
    let operation = Uuid::new_v4().to_string();
    let dir = h.root.path().join("remote-receipts").join(&phone.id);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{operation}.json"));
    let receipt = Receipt {
        digest: hash(&json!(["persona", "hello"]).to_string()),
        result: None,
    };
    atomic_write(&path, &serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert_eq!(
        phone
            .prompt(&operation, "persona", "hello", &[], None, None)
            .await
            .unwrap()["state"],
        "unknown"
    );
    assert!(
        phone
            .prompt(&operation, "persona", "changed", &[], None, None)
            .await
            .is_err()
    );
    let receipt = Receipt {
        result: Some(Ok(())),
        ..receipt
    };
    atomic_write(&path, &serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert_eq!(
        phone
            .prompt(&operation, "persona", "hello", &[], None, None)
            .await
            .unwrap()["state"],
        "accepted"
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_prompt_for_a_work_thread_is_said_there_and_is_its_own_operation() {
    let h = Harness::new().await;
    let grant = h.pair().await;
    let phone = h.remote.authenticate_v2(&grant.public).unwrap();
    let pair = ThreadId {
        kind: ThreadKind::Pair,
        key: "a|b".into(),
    };
    let refused = phone
        .prompt(
            &Uuid::new_v4().to_string(),
            "persona",
            "hi",
            &[],
            None,
            Some(&pair),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("work thread"), "{refused}");

    // Said in a side thread, it goes to the side thread and never starts the
    // main session; the room here has none, and says so.
    let side = ThreadId::side("s1");
    let operation = Uuid::new_v4().to_string();
    let sent = phone
        .prompt(&operation, "persona", "hi", &[], Some("e1"), Some(&side))
        .await
        .unwrap_err();
    assert!(sent.to_lowercase().contains("side thread"), "{sent}");
    // The same id cannot be reused for the same words in the main conversation.
    let reused = phone
        .prompt(&operation, "persona", "hi", &[], Some("e1"), None)
        .await
        .unwrap_err();
    assert!(reused.contains("another message"), "{reused}");
    h.remote.configure(false, network::ALL).await.unwrap();
}
