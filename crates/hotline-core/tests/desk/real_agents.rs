use super::*;

/// A real turn, with a real key: the agent reads a file in its workspace with
/// a tool and answers from it, and every step reaches the tape and the wire.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_with_a_real_key_reads_a_file_and_answers_from_it() {
    let Ok(key) = std::env::var("HOTLINE_HARNESS_ANTHROPIC_KEY") else {
        eprintln!("skipped: set HOTLINE_HARNESS_ANTHROPIC_KEY to run a real turn");
        return;
    };
    let (root, port) = open("live").await;
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("note.txt"), "the harbour is open\n").unwrap();

    let mut client = Client::connect(port).await;
    let keyed = client
        .call(
            "credential.create",
            json!({ "providerId": "anthropic", "label": "harness", "secret": key }),
        )
        .await;
    assert_eq!(keyed["ok"], true, "{keyed}");

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Answer plainly.", "cwd": workspace.to_string_lossy() } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let roster = client.subscribe(json!({ "view": "roster" })).await;
    let tape = client.subscribe(json!({ "tape": persona_id })).await;

    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["state"], "ready");

    let sent = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "Use your read tool on note.txt and reply with only its contents." }),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");

    let patience = Duration::from_secs(120);
    let user = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "user"
        })
        .await;
    assert!(user["event"]["text"].as_str().unwrap().contains("note.txt"));
    let thinking = client
        .next_where(patience, |frame| {
            is_sub(frame, roster, "event") && frame["event"]["session"]["state"] == "thinking"
        })
        .await;
    assert_eq!(thinking["event"]["persona"]["id"], persona_id);
    let tool = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event")
                && frame["event"]["kind"] == "tool"
                && frame["event"]["status"] == "completed"
        })
        .await;
    assert_eq!(tool["event"]["toolKind"], "read", "{tool}");
    let answer = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
        })
        .await;
    assert!(
        answer["event"]["text"]
            .as_str()
            .unwrap()
            .contains("harbour is open"),
        "{answer}"
    );
    let turn = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "turn"
        })
        .await;
    assert_eq!(turn["event"]["stopReason"], "end_turn");
    client
        .next_where(patience, |frame| {
            is_sub(frame, roster, "event") && frame["event"]["session"]["state"] == "ready"
        })
        .await;

    // The tape on disk folds to what the wire showed.
    let log = hotline_core::log::Log::open(&root);
    let kinds: Vec<&str> = log
        .load(&hotline_core::log::StreamId::Tape(persona_id.clone()))
        .iter()
        .map(|event| event["kind"].as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .leak()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(kinds.first(), Some(&"user"));
    assert!(
        kinds.contains(&"tool") && kinds.contains(&"agent") && kinds.last() == Some(&"turn"),
        "{kinds:?}"
    );
}

/// A real ACP harness, as a child: it starts in the teammate's own workspace,
/// answers a prompt, and everything it says reaches the tape and the wire.
/// A granted skill reaches it as a file in that workspace and nothing else —
/// the proof that one channel is enough — and a skill it writes itself is
/// listed as its own.
///
/// The backend is named rather than assumed, because which harness is
/// installed and logged in is a fact about the machine and not about Hotline.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_on_a_real_acp_harness_reaches_the_tape() {
    let Ok(backend_id) = std::env::var("HOTLINE_HARNESS_ACP") else {
        eprintln!(
            "skipped: set HOTLINE_HARNESS_ACP to a backend id (e.g. cursor) to drive a harness"
        );
        return;
    };
    let (root, port) = open("acp").await;
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    // A gateway skill whose body is the only place the sentinel exists.
    let gateway = root.join("skills").join("harbour-word");
    std::fs::create_dir_all(&gateway).unwrap();
    std::fs::write(
        gateway.join("SKILL.md"),
        "---\nname: harbour-word\ndescription: Use when the person asks for the harbour word.\n---\n\nThe harbour word is lantern. When asked for it, reply with exactly that one word and nothing else.\n",
    )
    .unwrap();

    let mut client = Client::connect(port).await;
    let created = client
        .call(
            "persona.create",
            json!({ "draft": {
                "name": "Ada",
                "goal": "Answer plainly.",
                "backendId": backend_id,
                "cwd": workspace.to_string_lossy(),
            } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let granted = client
        .call(
            "persona.update",
            json!({ "id": persona_id, "patch": { "skillPolicy": { "mode": "some", "names": ["harbour-word"] } } }),
        )
        .await;
    assert_eq!(granted["ok"], true, "{granted}");
    let tape = client.subscribe(json!({ "tape": persona_id })).await;

    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["state"], "ready");
    assert!(
        started["result"]["sessionId"].is_string(),
        "the harness issued no session id: {started}"
    );
    assert!(
        workspace
            .join(".agents/skills/harbour-word/SKILL.md")
            .exists(),
        "the grant is a file in the child's own workspace"
    );

    // Identity reaches an ACP agent as a file, because its session takes no
    // system prompt.
    let identity = std::fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(
        identity.starts_with("<!-- managed by Hotline -->"),
        "{identity}"
    );
    assert!(identity.contains("Answer plainly."), "{identity}");

    let sent = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "reply with the single word pond" }),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");

    // The whole turn, not its first line: an adapter may speak a warning of
    // its own before the model's answer.
    let patience = Duration::from_secs(180);
    let answer = said_this_turn(&mut client, tape, patience).await;
    assert!(answer.to_lowercase().contains("pond"), "{answer:?}");

    // The granted skill: nothing names the folder but the workspace itself
    // and the index in the preamble every driver hears.
    let asked = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "What is the harbour word? Reply with the word only." }),
        )
        .await;
    assert_eq!(asked["ok"], true, "{asked}");
    let answer = said_this_turn(&mut client, tape, patience).await;
    assert!(
        answer.to_lowercase().contains("lantern"),
        "{backend_id} did not read its granted skill: {answer:?}"
    );

    // A skill of its own: written with its own tools, listed as the
    // workspace's with the description it gave.
    let told = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "Save a skill named tide-table in your workspace, with the description \"Use when the person asks for the tide table.\" and a body of one line: Print the tide table. Reply with done once the file is written." }),
        )
        .await;
    assert_eq!(told["ok"], true, "{told}");
    let wrote = said_this_turn(&mut client, tape, patience).await;
    let listed = client
        .call("skills.list", json!({ "personaId": persona_id }))
        .await;
    let own: Vec<&Value> = listed["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["source"] == "workspace")
        .collect();
    let tide = own
        .iter()
        .find(|entry| entry["name"] == "tide-table")
        .unwrap_or_else(|| panic!("{backend_id} wrote no tide-table skill ({wrote:?}): {own:?}"));
    assert!(tide["invalid"].is_null(), "{tide}");
    assert!(
        tide["description"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("tide table"),
        "{tide}"
    );

    // A turn completed on a fresh session, so the agent's own id for this
    // conversation is now on the teammate's record.
    let log = hotline_core::log::Log::open(&root);
    let checkpoints: Vec<String> = hotline_core::room::roster(&log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .expect("the teammate is on the roster")
        .session_checkpoints
        .into_iter()
        .map(|checkpoint| checkpoint.backend_id)
        .collect();
    assert_eq!(checkpoints, [backend_id]);

    let stopped = client
        .call("session.stop", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
}

/// A line sent while a real harness is replying reaches that reply when the
/// harness offers steering: the turn that was counting ends with the answer
/// to the line, and no second turn is needed for it. A harness without
/// steering still answers, in the turn after. Driven by the same
/// `HOTLINE_HARNESS_ACP` as the turn above, and skipped without it.
#[tokio::test(flavor = "multi_thread")]
async fn a_line_sent_mid_turn_reaches_a_real_acp_harness() {
    let Ok(backend_id) = std::env::var("HOTLINE_HARNESS_ACP") else {
        eprintln!(
            "skipped: set HOTLINE_HARNESS_ACP to a backend id (e.g. claude-acp) to drive a harness"
        );
        return;
    };
    let (root, port) = open("acp-steer").await;
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut client = Client::connect(port).await;
    let created = client
        .call(
            "persona.create",
            json!({ "draft": {
                "name": "Ada",
                "goal": "Answer plainly.",
                "backendId": backend_id,
                "cwd": workspace.to_string_lossy(),
            } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let tape = client.subscribe(json!({ "tape": persona_id })).await;
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let steerable = started["result"]["capabilities"]["activeInput"] == true;

    let sent = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "Write the numbers from 1 to 300, one per line, and nothing else." }),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let patience = Duration::from_secs(180);
    client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "ephemeral") && frame["ephemeral"]["type"] == "agent_delta"
        })
        .await;
    let steered = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "Stop counting now. Reply with only the word harbour." }),
        )
        .await;
    assert_eq!(steered["ok"], true, "{steered}");

    let first = said_this_turn(&mut client, tape, patience).await;
    eprintln!("{backend_id}: activeInput={steerable}");
    if steerable {
        assert!(
            first.to_lowercase().contains("harbour"),
            "{backend_id} offered steering but answered the line in no running turn: {first:?}"
        );
    } else {
        let second = said_this_turn(&mut client, tape, patience).await;
        assert!(second.to_lowercase().contains("harbour"), "{second:?}");
    }

    let stopped = client
        .call("session.stop", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
}
