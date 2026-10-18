//! What a teammate is told before it is asked anything, and what each line
//! carries beside its words.

use super::*;

/// A teammate is told which extra folders it has, which it may change, and
/// who holds it to them; one with none is told nothing about folders.
#[test]
fn the_preamble_names_the_granted_folders_and_who_enforces_them() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("app");
    std::fs::create_dir(&path).unwrap();
    let mut ada = persona("ada");
    assert!(
        !preamble(&ada, Some(Reach::Workspace), None, &[]).contains("granted you these folders")
    );
    ada.folders = Some(vec![
        crate::contract::FolderGrant {
            path: path.to_string_lossy().into_owned(),
            writable: false,
        },
        crate::contract::FolderGrant {
            path: "/srv/shared".to_string(),
            writable: true,
        },
    ]);
    let walled = preamble(&ada, Some(Reach::Workspace), None, &[]);
    assert!(
        walled.contains(&format!(
            "{} (read only), /srv/shared (you may change it)",
            path.display()
        )),
        "{walled}"
    );
    assert!(walled.contains("your file tools and shell reach by absolute path"));
    assert!(walled.contains("A change inside a read-only one is refused."));
    let open = preamble(&ada, Some(Reach::Machine), None, &[]);
    assert!(open.contains("so this is not enforced"), "{open}");
    let child = preamble(&ada, None, None, &[]);
    assert!(child.contains("File operations delegated to Hotline through ACP may use them"));
    assert!(child.contains("your harness's own tools keep their own permissions"));
    assert!(!child.contains("Your computer has them"));
    ada.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: None,
        memory: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    let desk = preamble(&ada, Some(Reach::Workspace), None, &[]);
    // Only a folder still on this machine is mounted, so only it is named.
    assert!(
        desk.contains(&format!(
            "Your computer has them too, the same way: {} at /home/agent/folders/app.",
            path.display()
        )),
        "{desk}"
    );
}

/// The preamble is everything the agent would otherwise have to ask for.
#[test]
fn the_preamble_says_who_where_how_far_and_when() {
    let mut ada = persona("ada");
    ada.cwd = "/tmp/harbour".to_string();
    let walled = preamble(&ada, Some(Reach::Workspace), None, &[]);
    assert!(walled.contains("You are Ada."));
    assert!(walled.contains("Keep the harbour running."));
    assert!(walled.contains("Your working directory is /tmp/harbour."));
    assert!(walled.contains("a path that leaves it is refused"));
    // No date: the clock rides on each line, so the preamble is the same
    // prefix every day.
    assert!(walled.contains(CLOCK));
    assert!(!walled.contains(&Local::now().format("%B %Y").to_string()));
    // Skills are an index, not a body: the name, when to use it, and the
    // file to read, after the tool sentence and before the house style.
    assert!(walled.contains("\n- hotline-room: "));
    assert!(walled.contains(".agents/skills/hotline-room/SKILL.md"));
    let tools_at = walled.find("`search_thread`").unwrap();
    let skills_at = walled.find("You have skills").unwrap();
    let style_at = walled.find("Hotline shows your reply as chat").unwrap();
    assert!(tools_at < skills_at && skills_at < style_at, "{walled}");
    assert!(
        walled.contains("Hotline shows your reply as chat"),
        "both kinds of agent are told the house style: {walled}"
    );

    let open = preamble(
        &ada,
        Some(Reach::Machine),
        Some("the wake block".to_string()),
        &[],
    );
    assert!(open.contains("reach the whole machine"));
    assert!(open.ends_with("the wake block"));

    // The harness owns its tools' permissions; Hotline owns its file callbacks.
    let child = preamble(&ada, None, None, &[]);
    assert!(child.contains("Your working directory is /tmp/harbour."));
    assert!(!child.contains("reach the whole machine"));
    assert!(!child.contains("a path that leaves it is refused"));
    assert!(child.contains("Your harness manages the permissions of its own tools."));
    assert!(child.contains(
        "File operations delegated to Hotline through ACP stay inside your working directory."
    ));
    assert!(
        child.contains("Hotline shows your reply as chat"),
        "an ACP child hears the same house style in its preamble: {child}"
    );

    // A computer is granted outside the policy, so the preamble is where a
    // teammate learns it has one, and that the person can take it over.
    assert!(!child.contains("You have a computer"));
    ada.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: None,
        memory: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    let desk = preamble(&ada, Some(Reach::Workspace), None, &[]);
    assert!(desk.contains("You have a computer"));
    assert!(desk.contains("take it over"));
    assert!(desk.contains("action `guide`"));
    assert!(desk.contains("from that running computer"));
    assert!(desk.contains("`request_human`"));
    assert!(
        walled.contains("`request_human`"),
        "the preamble names the tool that asks the person: {walled}"
    );
}

/// A line carries the local day and time it was said, so a chapter that runs
/// past midnight hears the new day on the next line.
#[test]
fn a_line_says_the_day_and_time_it_was_said() {
    let before = Local
        .with_ymd_and_hms(2026, 9, 24, 23, 59, 0)
        .unwrap()
        .timestamp_millis();
    let after = before + 60_000;
    assert_eq!(
        timed(before, "still up?"),
        "[Thu 24 Sep 2026, 23:59] still up?"
    );
    assert_eq!(
        timed(after, "what day is it?"),
        "[Fri 25 Sep 2026, 00:00] what day is it?"
    );
}

/// A teammate naming a backend no agent on this machine answers to is told
/// so, rather than quietly started on a different agent than the one it names.
#[tokio::test]
async fn a_backend_no_agent_answers_to_is_refused_by_name() {
    let log = scratch("backend");
    let mut stranger = persona("stranger");
    stranger.backend_id = "nonesuch".to_string();
    enrol(&log, &stranger);
    let room = Room::new(log, Arc::new(DeskKeys));

    let refused = room.start("stranger").await.unwrap_err();
    assert!(refused.contains("nonesuch"), "{refused}");
}

/// The history a driver starts back into is what was said in the chapter it
/// is joining, and only that.
#[test]
fn the_conversation_a_driver_is_seeded_with_is_the_words_of_its_own_chapter() {
    let older = [
        json!({"kind": "user", "id": "u1", "ts": 1, "text": "hello"}),
        json!({"kind": "thought", "id": "t1", "ts": 2, "text": "hmm"}),
        json!({"kind": "tool", "id": "tool:c1", "ts": 3, "toolCallId": "c1", "title": "ls", "status": "completed"}),
        json!({"kind": "agent", "id": "a1", "ts": 4, "text": "hi"}),
    ];
    // Each of the person's lines comes back with the time it was said, as
    // the driver heard it live.
    let hello = [Said::User(timed(1, "hello")), Said::Agent("hi".to_string())];

    // A tape nobody has divided reads as one implicit chapter.
    assert_eq!(said(&older), hello);

    // An open chapter is the context: what came before it belongs to a
    // context that has already been summarised and let go of.
    let mut divided = older.to_vec();
    divided.push(json!({"kind": "chapter", "id": "c1", "ts": 5, "backendId": "hotline"}));
    divided.push(json!({"kind": "user", "id": "u2", "ts": 6, "text": "still there?"}));
    assert_eq!(said(&divided), [Said::User(timed(6, "still there?"))]);

    // The last chapter closed, so this session starts on nothing: the wake
    // block is what carries the chapter behind it.
    let mut closed = older.to_vec();
    closed.push(
        json!({"kind": "chapter", "id": "c1", "ts": 5, "backendId": "hotline", "endedAt": 9}),
    );
    assert_eq!(said(&closed), []);
}

/// Correlated deliveries must be admitted live before they become history;
/// read work and legacy, uncorrelated deliveries keep their existing context.
#[test]
fn seeded_history_only_remembers_read_correlated_deliveries() {
    let handoff = json!({
        "kind": "handoff", "requestId": "request", "personaId": "bob",
        "name": "Bob", "threadKey": "ada~bob", "about": "work"
    });
    let result = json!({
        "kind": "peer", "requestId": "request", "personaId": "bob",
        "name": "Bob", "threadKey": "ada~bob", "about": "work", "status": "done"
    });
    let mut legacy = result.clone();
    legacy.as_object_mut().unwrap().remove("requestId");
    for cause in [handoff, result, legacy] {
        for receipt in [None, Some("sent"), Some("read")] {
            let mut event = json!({
                "kind": "delivery", "id": "delivery", "ts": 1,
                "text": "Do this work", "cause": cause
            });
            if let Some(receipt) = receipt {
                event["receipt"] = json!(receipt);
            }
            let expected = if receipt == Some("read") || cause.get("requestId").is_none() {
                let cause = serde_json::from_value(cause.clone()).unwrap();
                vec![Said::User(timed(
                    1,
                    &peers::delivery_wire(&cause, "Do this work"),
                ))]
            } else {
                Vec::new()
            };
            assert_eq!(said(&[event]), expected, "{cause}, {receipt:?}");
        }
    }
}

/// Which of the person's apps wrote a line rides beside its time, for the
/// teammate only: the tape keeps the words as they were typed.
#[tokio::test]
async fn a_line_says_which_app_wrote_it_to_the_teammate_and_not_the_tape() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.heard.clone();
    let room = room("client", agents);
    room.start("ada").await.unwrap();
    crate::wire::commands::PROMPT_CLIENT
        .scope(
            crate::contract::Client::Phone,
            room.prompt("ada", "run it in the browser", None, None),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while lock(&prompts).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let heard = lock(&prompts)[0].clone();
    assert!(
        heard.ends_with(" · phone] run it in the browser"),
        "{heard}"
    );
    let line = tape(&room, "ada")
        .into_iter()
        .rev()
        .find(|event| event["kind"] == "user")
        .unwrap();
    assert_eq!(line["text"], "run it in the browser");
    assert_eq!(line["client"], "phone");
    // A restart rebuilds the history from the tape and keeps where it came from.
    let rebuilt = said(&tape(&room, "ada"));
    assert!(
        rebuilt
            .iter()
            .any(|line| matches!(line, Said::User(text) if text.contains(" · phone] run it"))),
        "{rebuilt:?}"
    );
}

#[test]
fn a_stamp_names_the_app_only_when_it_is_known() {
    let at = Local
        .with_ymd_and_hms(2026, 10, 2, 16, 20, 0)
        .unwrap()
        .timestamp_millis();
    assert_eq!(
        timed_from(at, Some(crate::contract::Client::Desktop), "open it"),
        "[Fri 2 Oct 2026, 16:20 · desktop] open it"
    );
    assert_eq!(timed_from(at, None, "open it"), timed(at, "open it"));
}
