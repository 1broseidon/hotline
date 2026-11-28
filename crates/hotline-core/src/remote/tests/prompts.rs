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
            .prompt(&operation, "persona", "hello", &[], None, None, None)
            .await
            .unwrap()["state"],
        "unknown"
    );
    assert!(
        phone
            .prompt(&operation, "persona", "changed", &[], None, None, None)
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
            .prompt(&operation, "persona", "hello", &[], None, None, None)
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
            None,
        )
        .await
        .unwrap_err();
    assert!(refused.contains("work thread"), "{refused}");
    // A follow-up on an inbox message is said in the conversation alone.
    let follow_up = phone
        .prompt(
            &Uuid::new_v4().to_string(),
            "persona",
            "hi",
            &[],
            None,
            Some(&ThreadId::side("s1")),
            Some("item-1"),
        )
        .await
        .unwrap_err();
    assert!(follow_up.contains("in the conversation"), "{follow_up}");

    // Said in a side thread, it goes to the side thread and never starts the
    // main session; the room here has none, and says so.
    let side = ThreadId::side("s1");
    let operation = Uuid::new_v4().to_string();
    let sent = phone
        .prompt(
            &operation,
            "persona",
            "hi",
            &[],
            Some("e1"),
            Some(&side),
            None,
        )
        .await
        .unwrap_err();
    assert!(sent.to_lowercase().contains("side thread"), "{sent}");
    // The same id cannot be reused for the same words in the main conversation.
    let reused = phone
        .prompt(&operation, "persona", "hi", &[], Some("e1"), None, None)
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
                .prompt(&operation, "persona", "hello", &[], None, None, None)
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
                    None,
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
        .prompt(&operation, "persona", "hello", &[], None, None, None)
        .await;
    assert!(room.starts.load(std::sync::atomic::Ordering::SeqCst) > before);
    assert_ne!(
        retried.ok().map(|v| v["state"].clone()),
        Some(json!("unknown"))
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}

/// A confined teammate on the desk at `cwd`, bound to the directory there
/// now, as a teammate is once it has started once.
#[cfg(unix)]
fn confined(h: &Harness, id: &str, name: &str, cwd: &std::path::Path) -> crate::contract::Persona {
    let mut persona = crate::session::tests::persona(id);
    persona.name = name.to_string();
    persona.reach = None;
    persona.cwd = cwd.to_string_lossy().into_owned();
    persona.cwd_identity = crate::tools::path_identity(cwd);
    crate::session::tests::enrol(&h.desk.log, &persona);
    persona
}

/// The error notices on a teammate's tape, in order.
#[cfg(unix)]
fn notices(h: &Harness, persona: &str) -> Vec<Value> {
    h.desk
        .log
        .load(&crate::log::StreamId::Tape(persona.into()))
        .into_iter()
        .filter(|event| event["kind"] == "notice" && event["level"] == "error")
        .collect()
}

/// A message from the phone to a teammate whose start is refused is said
/// back in the conversation the phone reads, in the refusal's own words,
/// because the phone gives a text-only message's words back with nothing
/// else to show and has no band: it is the notice's line, not the receipt's
/// `Err`, that tells the person. The window's send or Start says it the
/// same way, and saying it again adds nothing until something else is said.
#[cfg(unix)]
#[tokio::test]
async fn a_phone_message_to_a_teammate_that_cannot_start_says_why_in_its_conversation() {
    let h = Harness::new().await;
    let work = h.root.path().join("work");
    fs::create_dir(&work).unwrap();
    let ada = confined(&h, "ada", "Ada", &work);
    // The chosen folder renamed away and another made at its path.
    fs::rename(&work, h.root.path().join("work-was")).unwrap();
    fs::create_dir(&work).unwrap();
    let refusal = crate::tools::not_the_chosen_folder(&work);
    let said = format!("Ada did not start: {refusal}");

    let grant = h.pair().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    read(&mut phone).await;
    send(&mut phone, json!({"id":1,"sub":{"tape":ada.id}})).await;
    assert_eq!(read(&mut phone).await["ok"], true);
    assert_eq!(read(&mut phone).await["snapshot"], json!([]));
    send(&mut phone, json!({"id":2,"cmd":"mobile.prompt","params":{"operationId":Uuid::new_v4(),"personaId":ada.id,"text":"Are you there?","attachmentIds":[]}})).await;
    let (mut answered, mut told) = (false, false);
    while !answered || !told {
        let frame = read(&mut phone).await;
        if frame["id"] == 2 {
            assert_eq!(frame["ok"], false, "{frame}");
            assert_eq!(frame["error"], refusal.as_str(), "{frame}");
            answered = true;
        }
        if frame["sub"] == 1 && frame["event"]["kind"] == "notice" {
            assert_eq!(frame["event"]["level"], "error", "{frame}");
            assert_eq!(frame["event"]["text"], said.as_str(), "{frame}");
            told = true;
        }
    }
    let tape = h.desk.log.load(&crate::log::StreamId::Tape(ada.id.clone()));
    assert!(
        tape.iter().all(|event| event["kind"] != "user"),
        "a refused message is not written as said"
    );
    assert!(!said.contains("Err("), "{said}");

    // Sent again, from the phone and then from the window, it is refused
    // the same way, and the conversation still says it once.
    let again = h
        .remote
        .authenticate_v2(&grant.public)
        .unwrap()
        .prompt(
            &Uuid::new_v4().to_string(),
            &ada.id,
            "Hello?",
            &[],
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(again, refusal);
    let parts = h.desk.parts();
    let door = Door::bind(
        h.desk.log.clone(),
        "test-desk".into(),
        h.desk.clone(),
        parts,
    )
    .unwrap();
    let port = door.port();
    let task = tokio::spawn(door.run());
    let (mut desk, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?token=test-desk"))
            .await
            .unwrap();
    desk.send(Message::text(
        json!({"id":1,"cmd":"session.start","params":{"personaId":ada.id}}).to_string(),
    ))
    .await
    .unwrap();
    let started: Value = loop {
        let frame: Value =
            serde_json::from_str(desk.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        if frame["id"] == 1 {
            break frame;
        }
    };
    assert_eq!(started["ok"], false, "{started}");
    assert_eq!(started["error"], refusal.as_str(), "{started}");
    let told = notices(&h, &ada.id);
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0]["text"], said.as_str());
    h.remote.configure(false, network::ALL).await.unwrap();
    task.abort();
}

/// Every refusal of a confined teammate's workspace at its start reaches
/// the phone the same way: a working directory that became a link, a
/// granted folder that became one, and a folder not bound yet that a
/// confined teammate could have replaced.
#[cfg(unix)]
#[tokio::test]
async fn each_workspace_refusal_at_a_phone_message_is_said_in_the_conversation() {
    let h = Harness::new().await;
    let root = h.root.path().canonicalize().unwrap();
    // Linked: the working directory is a link now.
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    let linked = root.join("linked");
    std::os::unix::fs::symlink(&outside, &linked).unwrap();
    let mut lin = crate::session::tests::persona("lin");
    lin.name = "Lin".into();
    lin.reach = None;
    lin.cwd = linked.to_string_lossy().into_owned();
    crate::session::tests::enrol(&h.desk.log, &lin);
    // Granted again: a folder it was granted is a link now.
    let gra_work = root.join("gra");
    fs::create_dir(&gra_work).unwrap();
    fs::create_dir(root.join("target")).unwrap();
    let granted = root.join("granted");
    std::os::unix::fs::symlink(root.join("target"), &granted).unwrap();
    let mut gra = confined(&h, "gra", "Gra", &gra_work);
    gra.folders = Some(vec![crate::contract::FolderGrant {
        path: granted.to_string_lossy().into_owned(),
        writable: true,
        identity: None,
    }]);
    crate::session::tests::enrol(&h.desk.log, &gra);
    // Unbound: its folder lies inside one Bob, confined, may write.
    let holder = root.join("holder");
    fs::create_dir_all(holder.join("mid/work")).unwrap();
    let mut bob = confined(&h, "bob", "Bob", &root.join("bob-work"));
    bob.folders = Some(vec![crate::contract::FolderGrant {
        path: holder.to_string_lossy().into_owned(),
        writable: true,
        identity: None,
    }]);
    crate::session::tests::enrol(&h.desk.log, &bob);
    let mut unb = crate::session::tests::persona("unb");
    unb.name = "Unb".into();
    unb.reach = None;
    unb.cwd = holder.join("mid/work").to_string_lossy().into_owned();
    crate::session::tests::enrol(&h.desk.log, &unb);

    let grant = h.pair().await;
    let phone = h.remote.authenticate_v2(&grant.public).unwrap();
    for (id, name, why) in [
        ("lin", "Lin", "is now a link to"),
        ("gra", "Gra", "Grant the folder again"),
        ("unb", "Unb", "Choose the folder again"),
    ] {
        let refused = phone
            .prompt(&Uuid::new_v4().to_string(), id, "hi", &[], None, None, None)
            .await
            .unwrap_err();
        assert!(refused.contains(why), "{id}: {refused}");
        let told = notices(&h, id);
        assert_eq!(told.len(), 1, "{id}: {told:?}");
        assert_eq!(
            told[0]["text"],
            format!("{name} did not start: {refused}").as_str(),
            "{id}"
        );
    }
    h.remote.configure(false, network::ALL).await.unwrap();
}
