use super::*;

/// Claude's adapter marks the call that launched a subagent only in
/// `_meta`; kept as a launch, a turn left open for it reads as done.
#[test]
fn a_call_claude_says_launched_a_subagent_is_kept_as_one() {
    let meta = |name: &str| {
        serde_json::json!({"claudeCode": {"toolName": name}})
            .as_object()
            .cloned()
            .unwrap()
    };
    assert_eq!(
        tool_kind(acp::ToolKind::Other, Some(&meta("Agent"))),
        SUBAGENT_LAUNCH
    );
    assert_eq!(
        tool_kind(acp::ToolKind::Think, Some(&meta("Task"))),
        SUBAGENT_LAUNCH
    );
    assert_eq!(
        tool_kind(acp::ToolKind::Execute, Some(&meta("Bash"))),
        "execute"
    );
    assert_eq!(tool_kind(acp::ToolKind::Read, None), "read");
}
fn recovery_agent(
    heard: Heard,
    failure: &'static str,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let _ = agent_client_protocol::Agent
            .builder()
            .name("recovery-fixture")
            .on_receive_request(
                async |request: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_capabilities(AgentCapabilities::new().load_session(true)),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: LoadSessionRequest,
                       responder: Responder<LoadSessionResponse>,
                       _cx| {
                    responder.respond_with_internal_error("Unknown session")
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("fresh-fixture")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let heard = heard.clone();
                    async move {
                        heard.prompted.lock().unwrap().push(
                            request
                                .prompt
                                .iter()
                                .filter_map(|block| match block {
                                    ContentBlock::Text(text) => Some(text.text.clone()),
                                    _ => None,
                                })
                                .collect(),
                        );
                        if failure == "unknown" {
                            return responder.respond_with_internal_error("Unknown session");
                        }
                        if failure == "tool" {
                            cx.send_notification(SessionNotification::new(
                                request.session_id,
                                SessionUpdate::ToolCall(
                                    ToolCall::new(
                                        ToolCallId::new("uncertain-call"),
                                        "write deployment record",
                                    )
                                    .status(ToolCallStatus::InProgress),
                                ),
                            ))?;
                        }
                        if !failure.is_empty() {
                            std::future::pending::<()>().await;
                        }
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
    }
}

#[tokio::test]
async fn failed_acp_sessions_get_a_fresh_briefing_without_reissuing_the_old_prompt() {
    for failure in ["unknown", "dead", "tool"] {
        let held = room(&format!("recovery-{failure}"));
        let driver = ChildAgent::new(
            scratch(failure),
            "cursor".into(),
            "you are Ada".into(),
            TeammateTools::new(&held, "ada"),
        )
        .with_history(vec![crate::driver::rig::Said::User(
            "Earlier conversation".into(),
        )]);
        let heard = Heard::default();
        let agent = tokio::spawn(recovery_agent(heard.clone(), failure));
        let ada = persona(
            &scratch_cwd(),
            vec![SessionCheckpoint {
                backend_id: "cursor".into(),
                session_id: "stale".into(),
                account_id: None,
            }],
        );
        let info = driver.handshake(&ada, client_transport()).await.unwrap();
        assert!(!info.context_restored, "unknown saved sessions start fresh");
        let mut updates = driver
            .prompt("original request".into(), vec![], Reach::Workspace)
            .await;
        if failure == "tool" {
            assert!(matches!(next(&mut updates).await, Update::ToolCall { .. }));
        }
        if failure != "unknown" {
            tokio::time::timeout(Duration::from_secs(3), async {
                while heard.prompted.lock().unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            agent.abort();
        }
        let mut failed = false;
        while let Some(update) = updates.recv().await {
            if matches!(update, Update::Turn { stop_reason, .. } if stop_reason == "failed") {
                failed = true;
            }
        }
        assert!(failed);
        assert!(!driver.checkpoint_valid());
        assert!(!driver.live.briefed.load(Ordering::SeqCst));
        assert!(lock(&driver.live.pending).is_empty());
        let fresh_persona = driver.abandon_failed_session().await.unwrap();
        assert_eq!(
            fresh_persona.session_checkpoints,
            [] as [SessionCheckpoint; 0]
        );
        assert!(lock(&driver.live.connection_task).is_none());
        let fresh_heard = Heard::default();
        let fresh_agent = tokio::spawn(recovery_agent(fresh_heard.clone(), ""));
        driver
            .handshake(&fresh_persona, client_transport())
            .await
            .unwrap();
        driver.live.failed.store(false, Ordering::SeqCst);
        let mut updates = driver
            .prompt("check the outcome".into(), vec![], Reach::Workspace)
            .await;
        assert!(
            matches!(next(&mut updates).await, Update::Turn { stop_reason, .. } if stop_reason == "end_turn")
        );
        let prompts = fresh_heard.prompted.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].last().unwrap(), "check the outcome");
        let brief = prompts[0].join("\n");
        assert!(brief.contains("you are Ada"));
        assert!(brief.contains("Earlier conversation"));
        assert_eq!(brief.matches("original request").count(), 1);
        assert!(brief.contains("do not automatically repeat"));
        if failure == "tool" {
            assert!(brief.contains("uncertain-call"));
        }
        agent.abort();
        fresh_agent.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn abandoning_an_acp_session_waits_for_the_owned_child_to_exit() {
    let held = room("abandon-child");
    let driver = ChildAgent::new(
        scratch("abandon"),
        "cursor".into(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    *lock(&driver.persona) = Some(persona(&scratch_cwd(), vec![]));
    let child = tokio::process::Command::new("sh")
        .args(["-c", "sleep 600"])
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    *lock(&driver.child) = Some(child);
    driver.abandon_failed_session().await.unwrap();
    assert_eq!(
        // SAFETY: signal 0 only asks whether the captured pid exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) },
        -1,
        "child must be reaped before another prompt can be sent"
    );
}

/// One turn end to end: what the agent streams becomes the room's
/// vocabulary, in the order it happened, and the permission in the middle
/// blocks the tool until it is answered.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_becomes_updates_in_the_order_the_agent_sent_them() {
    let root = scratch("turn");
    let heard = Heard::default();
    let agent = scripted_agent(heard.clone(), false);
    let held = room("turn-room");
    let ada = persona(&scratch_cwd(), Vec::new());
    let briefing = crate::session::preamble(&ada, None, None, &[]);
    let driver = ChildAgent::new(
        root,
        "cursor".to_string(),
        briefing.clone(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);

    let mut info_updates = driver.subscribe_info().unwrap();
    let info = driver.handshake(&ada, client_transport()).await.unwrap();
    assert_eq!(*info_updates.borrow_and_update(), info);
    // A listed backend goes by its listed name, not the one it reports.
    assert_eq!(info.agent_name, "Cursor");
    assert_eq!(info.agent_version.as_deref(), Some("1.2.3"));
    assert_eq!(info.session_id.as_deref(), Some("fresh-session"));
    assert!(!info.context_restored);
    assert_eq!(
        info.modes
            .iter()
            .map(|mode| mode.id.as_str())
            .collect::<Vec<_>>(),
        ["ask", "agent"]
    );
    assert_eq!(info.current_mode_id.as_deref(), Some("ask"));
    assert_eq!(heard.opened.lock().unwrap().as_slice(), ["session/new"]);

    let mut updates = driver
        .prompt(
            "how is the harbour".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;

    assert!(matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "on it"));
    assert!(
        matches!(next(&mut updates).await, Update::Message { text, kind, .. }
            if text == "on it" && kind == MessageKind::Agent)
    );
    let Update::ToolCall {
        call_id,
        title,
        kind,
    } = next(&mut updates).await
    else {
        panic!("the tool call did not arrive");
    };
    assert_eq!(
        (call_id.as_str(), title.as_str(), kind.as_str()),
        ("c1", "read harbour.log", "read")
    );

    let Update::Permission {
        request_id,
        options,
        ..
    } = next(&mut updates).await
    else {
        panic!("the permission did not arrive");
    };
    assert_eq!(
        options
            .iter()
            .map(|option| option.name.as_str())
            .collect::<Vec<_>>(),
        ["Allow once", "Deny"]
    );
    // An agent that never offered steering takes nothing mid-turn; the
    // session keeps the line for after.
    assert!(!info.capabilities.active_input);
    assert!(!driver.steer("and the north pier".to_string(), Vec::new()));
    assert!(driver.answer_permission(&request_id, "once"));
    // Answered once and only once: the second answer has nothing behind it.
    assert!(!driver.answer_permission(&request_id, "once"));

    assert!(matches!(
        next(&mut updates).await,
        Update::ToolResult { ok: true, output, .. } if output == "all clear"
    ));
    assert!(matches!(
        next(&mut updates).await,
        Update::Turn { stop_reason, .. } if stop_reason == "end_turn"
    ));

    // The briefing rides ahead of the first words and is never said again.
    let prompted = heard.prompted.lock().unwrap().clone();
    assert_eq!(prompted[0][0], briefing);
    assert!(
        prompted[0][0].contains("Hotline shows your reply as chat"),
        "the first prompt carries the house style: {}",
        prompted[0][0]
    );
    assert_eq!(prompted[0][1], "how is the harbour");
    assert_eq!(
        prompted[0].len(),
        2,
        "house style is in the preamble, not a second block"
    );
}

/// An agent that shows its subagents as sessions of their own: it is
/// told Hotline takes them, and announces one mid-turn whose work never
/// reaches the conversation's turn.
fn subagent_agent(
    capabilities: Arc<Mutex<Option<Value>>>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let running = agent_client_protocol::Agent
            .builder()
            .name("subagents")
            .on_receive_request(
                async move |request: RawInitialize, responder: Responder<InitializeResponse>, _cx| {
                    *capabilities.lock().unwrap() = request.0.get("clientCapabilities").cloned();
                    responder.respond(
                        InitializeResponse::new(ProtocolVersion::V1)
                            .agent_info(Implementation::new("subagents", "1.0.0")),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: NewSessionRequest,
                            responder: Responder<NewSessionResponse>,
                            _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("parent")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: PromptRequest,
                            responder: Responder<PromptResponse>,
                            cx: ConnectionTo<Client>| {
                    let say = |session: &str, update: Value| {
                        cx.send_notification(RawSessionUpdate {
                            session_id: session.to_string(),
                            update,
                        })
                    };
                    say("parent", serde_json::json!({
                        "sessionUpdate": "subagent_spawned",
                        "subagentSessionId": "child-1",
                        "name": "Test investigator",
                        "task": "Check the tests",
                        "prompt": "Run the tests and report.",
                        "capabilities": {},
                    }))?;
                    say("child-1", serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "Running them." },
                    }))?;
                    say("child-1", serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "c9",
                        "title": "cargo test",
                        "kind": "execute",
                        "status": "in_progress",
                    }))?;
                    say("child-1", serde_json::json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "c9",
                        "status": "completed",
                        "content": [{ "type": "content", "content": { "type": "text", "text": "3 passed" } }],
                    }))?;
                    say("child-1", serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "All 3 pass." },
                    }))?;
                    // A picker on the child is the child's own.
                    say("child-1", serde_json::json!({
                        "sessionUpdate": "current_mode_update",
                        "currentModeId": "child-mode",
                    }))?;
                    say("parent", serde_json::json!({
                        "sessionUpdate": "subagent_state_update",
                        "subagentSessionId": "child-1",
                        "state": "completed",
                    }))?;
                    say("parent", serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "They pass." },
                    }))?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the subagent agent ended: {error}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subagent_session_is_reported_apart_from_the_turn() {
    let capabilities = Arc::new(Mutex::new(None));
    let agent = subagent_agent(capabilities.clone());
    let held = room("subagent-room");
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch("subagents"),
        "claude".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);
    let mut reports = Driver::subscribe_subagents(&driver).unwrap();
    driver.handshake(&ada, client_transport()).await.unwrap();
    assert_eq!(
        capabilities.lock().unwrap().as_ref().unwrap()["subagents"],
        serde_json::json!({}),
        "Hotline says it takes subagents as sessions"
    );

    let mut updates = driver
        .prompt(
            "are the tests green".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;
    let mut turn = Vec::new();
    loop {
        let update = next(&mut updates).await;
        let done = matches!(update, Update::Turn { .. });
        turn.push(update);
        if done {
            break;
        }
    }
    assert_eq!(
        turn.iter()
            .filter_map(|update| match update {
                Update::Message { text, .. } => Some(text.as_str()),
                Update::ToolCall { .. } | Update::ToolResult { .. } => Some("tool"),
                _ => None,
            })
            .collect::<Vec<_>>(),
        ["They pass."],
        "the turn holds only the conversation's own words: {turn:?}"
    );
    assert_eq!(
        driver.live.session.lock().unwrap().info.current_mode_id,
        None
    );

    let mut reported = Vec::new();
    while let Ok(report) = reports.try_recv() {
        reported.push(report);
    }
    let child = || "child-1".to_string();
    assert_eq!(
        reported,
        [
            SubagentReport::Started {
                child: child(),
                title: "Test investigator".to_string(),
                task: "Run the tests and report.".to_string(),
            },
            SubagentReport::Update {
                child: child(),
                update: Update::Message {
                    kind: MessageKind::Agent,
                    id: match &reported[1] {
                        SubagentReport::Update {
                            update: Update::Message { id, .. },
                            ..
                        } => id.clone(),
                        other => panic!("not the first words: {other:?}"),
                    },
                    text: "Running them.".to_string(),
                },
            },
            SubagentReport::Update {
                child: child(),
                update: Update::ToolCall {
                    call_id: "c9".to_string(),
                    title: "cargo test".to_string(),
                    kind: "execute".to_string(),
                },
            },
            SubagentReport::Update {
                child: child(),
                update: Update::ToolResult {
                    call_id: "c9".to_string(),
                    ok: true,
                    output: "3 passed".to_string(),
                    images: Vec::new(),
                },
            },
            SubagentReport::Update {
                child: child(),
                update: Update::Message {
                    kind: MessageKind::Agent,
                    id: match &reported[4] {
                        SubagentReport::Update {
                            update: Update::Message { id, .. },
                            ..
                        } => id.clone(),
                        other => panic!("not the last words: {other:?}"),
                    },
                    text: "All 3 pass.".to_string(),
                },
            },
            SubagentReport::Ended {
                child: child(),
                status: SubagentStatus::Done,
            },
        ]
    );
}

/// An agent that answers in one cycle, closes it with the usage report
/// that carries its cost, and holds the prompt open until `release`, as
/// Claude's adapter holds it for its background subagents. With `spawns`
/// it started one first, which ends on release and is followed by one
/// more cycle of the agent's own words.
fn holding_agent(
    spawns: bool,
    release: Arc<tokio::sync::Notify>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let running = agent_client_protocol::Agent
            .builder()
            .name("holding")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    responder.respond(InitializeResponse::new(request.protocol_version))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: NewSessionRequest,
                            responder: Responder<NewSessionResponse>,
                            _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("parent")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |_request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let release = release.clone();
                    async move {
                        let later = cx.clone();
                        cx.spawn(async move {
                            let say = |update: Value| {
                                later.send_notification(RawSessionUpdate {
                                    session_id: "parent".to_string(),
                                    update,
                                })
                            };
                            let closes = || {
                                serde_json::json!({
                                    "sessionUpdate": "usage_update",
                                    "used": 1000,
                                    "size": 200_000,
                                    "cost": { "amount": 0.01, "currency": "USD" },
                                })
                            };
                            if spawns {
                                say(serde_json::json!({
                                    "sessionUpdate": "subagent_spawned",
                                    "subagentSessionId": "child-1",
                                    "name": "Edge cases",
                                    "task": "Check the edge cases",
                                    "capabilities": {},
                                }))?;
                            }
                            say(serde_json::json!({
                                "sessionUpdate": "agent_message_chunk",
                                "content": { "type": "text", "text": "The lift is in crane.rs." },
                            }))?;
                            say(closes())?;
                            release.notified().await;
                            if spawns {
                                say(serde_json::json!({
                                    "sessionUpdate": "subagent_state_update",
                                    "subagentSessionId": "child-1",
                                    "state": "completed",
                                }))?;
                                say(serde_json::json!({
                                    "sessionUpdate": "agent_message_chunk",
                                    "content": { "type": "text", "text": "The edge cases pass." },
                                }))?;
                                say(closes())?;
                            }
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })?;
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the holding agent ended: {error}");
        }
    }
}

/// The agent's words as the turn hears them: deltas left out, a message
/// by its text, a park and the turn's end by name.
fn heard_of(update: &Update) -> Option<String> {
    match update {
        Update::Message { text, .. } => Some(text.clone()),
        Update::Parked => Some("parked".to_string()),
        Update::Turn { stop_reason, .. } => Some(stop_reason.clone()),
        _ => None,
    }
}

/// A prompt held open for a subagent: once the agent closes the cycle its
/// reply was in, the reply is a whole message and the turn parks, and
/// what it says once the subagent is done is a reply of its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_held_for_a_subagent_parks_on_the_finished_reply() {
    let release = Arc::new(tokio::sync::Notify::new());
    let agent = holding_agent(true, release.clone());
    let held = room("held-for-subagent");
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch("held-for-subagent"),
        "claude".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);
    let _reports = Driver::subscribe_subagents(&driver).unwrap();
    driver.handshake(&ada, client_transport()).await.unwrap();
    let mut updates = driver
        .prompt(
            "where is the lift".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;
    let mut heard = Vec::new();
    while heard.last().map(String::as_str) != Some("parked") {
        heard.extend(heard_of(&next(&mut updates).await));
    }
    assert_eq!(heard, ["The lift is in crane.rs.", "parked"]);

    release.notify_one();
    let mut after = Vec::new();
    while after.last().map(String::as_str) != Some("end_turn") {
        after.extend(heard_of(&next(&mut updates).await));
    }
    assert_eq!(
        after,
        ["The edge cases pass.", "end_turn"],
        "the last subagent is done, so the closing cycle does not park"
    );
}

/// An agent with no subagent open that closes a cycle and keeps the
/// prompt a moment longer is a turn like any other: its words stay open
/// until the turn ends, and nothing parks.
#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_with_no_subagent_open_never_parks() {
    let release = Arc::new(tokio::sync::Notify::new());
    let agent = holding_agent(false, release.clone());
    let held = room("held-without-subagent");
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch("held-without-subagent"),
        "claude".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);
    let _reports = Driver::subscribe_subagents(&driver).unwrap();
    driver.handshake(&ada, client_transport()).await.unwrap();
    let mut updates = driver
        .prompt(
            "where is the lift".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;
    assert!(
        matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "The lift is in crane.rs.")
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        updates.try_recv().is_err(),
        "nothing more until the turn ends"
    );
    release.notify_one();
    let mut heard = Vec::new();
    while heard.last().map(String::as_str) != Some("end_turn") {
        heard.extend(heard_of(&next(&mut updates).await));
    }
    assert_eq!(heard, ["The lift is in crane.rs.", "end_turn"]);
}

#[test]
fn announcements_are_read_from_the_adapters_draft_updates() {
    assert_eq!(
        announced(&serde_json::json!({
            "sessionUpdate": "subagent_spawned",
            "subagentSessionId": "t1:generation:2",
            "name": "",
            "task": "Explore the repo",
            "capabilities": {},
        })),
        Some(Announced::Spawned {
            child: "t1:generation:2".to_string(),
            title: "Subagent".to_string(),
            task: "Explore the repo".to_string(),
        })
    );
    for (state, status) in [
        ("completed", SubagentStatus::Done),
        ("failed", SubagentStatus::Failed),
        ("cancelled", SubagentStatus::Cancelled),
        ("disconnected", SubagentStatus::Cancelled),
    ] {
        assert_eq!(
            announced(&serde_json::json!({
                "sessionUpdate": "subagent_state_update",
                "subagentSessionId": "c",
                "state": state,
            })),
            Some(Announced::Ended {
                child: "c".to_string(),
                status,
            })
        );
    }
    assert_eq!(
        announced(&serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hi" },
        })),
        None
    );
}

/// A teammate with a checkpoint for this backend rejoins its own session,
/// and the history the load replays is not written down a second time.
#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_reopens_the_agents_own_session_without_replaying_it() {
    let root = scratch("load");
    let heard = Heard::default();
    let agent = scripted_agent(heard.clone(), true);
    let held = room("load-room");
    let driver = ChildAgent::new(
        root,
        "cursor".to_string(),
        "you are Ada".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);

    let ada = persona(
        &scratch_cwd(),
        vec![SessionCheckpoint {
            backend_id: "cursor".to_string(),
            session_id: "old-session".to_string(),
            account_id: None,
        }],
    );
    let info = driver.handshake(&ada, client_transport()).await.unwrap();
    assert_eq!(info.session_id.as_deref(), Some("old-session"));
    assert!(info.context_restored);
    assert_eq!(
        heard.opened.lock().unwrap().as_slice(),
        ["session/load old-session"]
    );

    // Nothing the replay said reached a turn: the first update of the
    // first turn is that turn's own first word.
    let mut updates = driver
        .prompt("carry on".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert!(matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "on it"));
}

/// Ending a session ends the child, and everything the child started.
///
/// The connection is kept alive by the child's own pipes, so nothing but
/// the driver going away can close it — which is why the handle lives on
/// the driver and not beside the connection.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_driver_takes_the_childs_whole_process_group() {
    let held = room("kill-room");
    let driver = ChildAgent::new(
        scratch("kill"),
        "cursor".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    let mut command = tokio::process::Command::new("sh");
    command
        .args(["-c", "sleep 600"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    let child = command.spawn().unwrap();
    let group = child.id().expect("the child has a pid") as libc::pid_t;
    *lock(&driver.child) = Some(child);
    // SAFETY: `kill` with signal 0 asks whether the group exists.
    assert_eq!(unsafe { libc::killpg(group, 0) }, 0, "the group is running");

    drop(driver);
    for _ in 0..200 {
        // SAFETY: as above, signal 0 only asks.
        if unsafe { libc::killpg(group, 0) } != 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the child's process group outlived its driver");
}

/// An ACP agent that raises a permission and then ends the turn without
/// waiting for the answer — every harness does this when it gives up on a
/// request — and does not end it until the test says so, so the card is up
/// and drawn before the turn is over.
fn agent_that_ends_the_turn_still_asking(
    ends: Arc<tokio::sync::Notify>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let running = agent_client_protocol::Agent
            .builder()
            .name("still-asking")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_info(Implementation::new("still-asking", "1")),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("asking")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let ends = ends.clone();
                    async move {
                        let session = request.session_id.clone();
                        let asking = cx.clone();
                        cx.spawn(async move {
                            let _ = asking
                                .send_request(RequestPermissionRequest::new(
                                    session,
                                    ToolCallUpdate::new(
                                        ToolCallId::new("c1"),
                                        ToolCallUpdateFields::new(),
                                    ),
                                    vec![acp::PermissionOption::new(
                                        "once",
                                        "Allow once",
                                        PermissionOptionKind::AllowOnce,
                                    )],
                                ))
                                .block_task()
                                .await;
                            Ok(())
                        })?;
                        cx.spawn(async move {
                            ends.notified().await;
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        })?;
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the still-asking agent ended: {error}");
        }
    }
}

/// A card is refused from the moment the agent ends the turn, not from the
/// moment the transcript catches up with it.
///
/// `docs/sessions.md`: a stale card — the turn ended, the session stopped,
/// somebody else answered first — is refused, so the transcript never
/// shows a decision the agent never heard. The flush and the turn event
/// after `session/prompt` returns are Hotline writing down what already
/// happened; a permission answered while it does is answered into nothing.
/// Here the update channel is filled so that writing-down is wedged, which
/// leaves the end of the turn as the only thing that can settle the card.
#[tokio::test(flavor = "multi_thread")]
async fn a_card_is_refused_from_the_moment_the_agent_ends_the_turn() {
    let held = room("stale-card-room");
    let ends = Arc::new(tokio::sync::Notify::new());
    let agent = agent_that_ends_the_turn_still_asking(ends.clone());
    let driver = ChildAgent::new(
        scratch("stale-card"),
        "cursor".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);
    driver
        .handshake(&persona(&scratch_cwd(), Vec::new()), client_transport())
        .await
        .unwrap();

    let mut updates = driver
        .prompt("go".to_string(), Vec::new(), Reach::Workspace)
        .await;
    let Update::Permission { request_id, .. } = next(&mut updates).await else {
        panic!("the card did not arrive");
    };

    // Every place in the update channel is taken, so nothing the driver
    // writes down after the turn can leave it. What settles the card now
    // is the end of the turn or nothing at all.
    let sender = lock(&driver.live.updates)
        .clone()
        .expect("the turn is running");
    for _ in 0..UPDATE_DEPTH {
        sender
            .try_send(Update::Notice {
                level: NoticeLevel::Info,
                text: "the channel is full".to_string(),
            })
            .expect("the channel takes its whole depth");
    }
    ends.notify_one();

    // A bounded wait for the agent's answer to cross the pipe; the
    // assertion below is what is being tested.
    for _ in 0..200 {
        if lock(&driver.live.pending).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !driver.answer_permission(&request_id, "once"),
        "a card the turn left behind was still answerable after the turn ended"
    );
}

#[test]
fn an_image_content_block_becomes_a_placeholder_and_is_carried() {
    let content = vec![
        acp::ToolCallContent::Content(acp::Content::new(ContentBlock::Text(TextContent::new(
            "the tree",
        )))),
        acp::ToolCallContent::Content(acp::Content::new(ContentBlock::Image(
            acp::ImageContent::new("AAAA", "image/png"),
        ))),
    ];
    let (ok, output, images) = finished(ToolCallStatus::Completed, &content).unwrap();
    assert!(ok);
    assert!(output.contains("the tree"), "{output}");
    assert!(output.contains("[image image/png, 3 B]"), "{output}");
    assert_eq!(
        images,
        [ToolImage {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
        }]
    );
}
