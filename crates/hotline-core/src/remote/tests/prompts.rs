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
    let pair = ThreadId::new(ThreadKind::Pair, "a|b");
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

#[tokio::test]
async fn a_prompt_dropped_before_it_is_dispatched_leaves_no_receipt_and_a_hung_start_blocks_no_other_phone()
 {
    let (h, room) = Harness::with_computer().await;
    let grant = h.pair().await;
    let phone = h.remote.authenticate_v2(&grant.public).unwrap();
    let (entered, release) = (CancellationToken::new(), CancellationToken::new());
    *room.start_gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let operation = Uuid::new_v4().to_string();
    let hung = {
        let phone = phone.clone();
        let operation = operation.clone();
        tokio::spawn(async move {
            phone
                .prompt(&operation, "persona", "hello", &[], None, None)
                .await
        })
    };
    tokio::time::timeout(ANSWER, entered.cancelled())
        .await
        .unwrap();
    // One start that never finishes does not hold up another operation.
    let other = {
        let phone = phone.clone();
        tokio::spawn(async move {
            phone
                .prompt(
                    &Uuid::new_v4().to_string(),
                    "persona",
                    "other",
                    &[],
                    None,
                    Some(&ThreadId::side("s1")),
                )
                .await
        })
    };
    let other = tokio::time::timeout(PROMPTLY, other).await;
    assert!(other.is_ok(), "a hung start held every phone's prompts");
    // The phone drops while the start is awaited: nothing was dispatched.
    hung.abort();
    let _ = hung.await;
    let receipt = h
        .root
        .path()
        .join("remote-receipts")
        .join(&phone.id)
        .join(format!("{operation}.json"));
    assert!(!receipt.exists(), "a receipt was written before dispatch");
    // The retry with the same id sends now; it is never answered "unknown".
    release.cancel();
    let before = room.starts.load(std::sync::atomic::Ordering::SeqCst);
    let retried = phone
        .prompt(&operation, "persona", "hello", &[], None, None)
        .await;
    assert!(room.starts.load(std::sync::atomic::Ordering::SeqCst) > before);
    assert_ne!(
        retried.ok().map(|v| v["state"].clone()),
        Some(json!("unknown"))
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}
