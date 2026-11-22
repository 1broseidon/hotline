use super::*;

/// An agent that answers its first prompt at once and then, with no
/// prompt out, reports on work it left running — ending that report with
/// a usage update when `reports` says so, as Claude does. Later prompts
/// are answered plainly.
fn autonomous_agent(
    reports: bool,
    steering: Option<Arc<Mutex<Vec<Value>>>>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let prompts = Arc::new(AtomicUsize::new(0));
        let advertised = steering.is_some();
        let running = agent_client_protocol::Agent
            .builder()
            .name("autonomous")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    let mut response = InitializeResponse::new(request.protocol_version);
                    if advertised {
                        let mut meta = acp::Meta::new();
                        meta.insert("steering".into(), serde_json::json!({"supported": true}));
                        response = response.meta(meta);
                    }
                    responder.respond(response)
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: NewSessionRequest,
                       responder: Responder<NewSessionResponse>,
                       _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("background")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let first = prompts.fetch_add(1, Ordering::SeqCst) == 0;
                    async move {
                        let session = request.session_id.clone();
                        cx.send_notification(SessionNotification::new(
                            session.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(if first {
                                    "started the build"
                                } else {
                                    "hello again"
                                })),
                            )),
                        ))?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))?;
                        if first {
                            let later = cx.clone();
                            cx.spawn(async move {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                let say = |update| {
                                    later.send_notification(SessionNotification::new(
                                        session.clone(),
                                        update,
                                    ))
                                };
                                say(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new("the build")),
                                )))?;
                                // A mid-cycle reading, as after each model call:
                                // no cost, so the cycle goes on.
                                say(SessionUpdate::UsageUpdate(acp::UsageUpdate::new(
                                    900, 200_000,
                                )))?;
                                say(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new(" passed")),
                                )))?;
                                if reports {
                                    say(SessionUpdate::UsageUpdate(
                                        acp::UsageUpdate::new(1_000, 200_000)
                                            .cost(acp::Cost::new(0.01, "USD")),
                                    ))?;
                                }
                                Ok(())
                            })?;
                        }
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: agent_client_protocol::UntypedMessage,
                      responder: Responder<Value>,
                      _cx| {
                    let steering = steering.clone();
                    async move {
                        assert_eq!(request.method, STEERING_METHOD);
                        lock(steering.as_deref().expect("steering was not advertised"))
                            .push(request.params);
                        responder.respond(serde_json::json!({ "outcome": "promptRequired" }))
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the autonomous agent ended: {error}");
        }
    }
}

async fn autonomous_driver(
    name: &str,
    reports: bool,
    steering: Option<Arc<Mutex<Vec<Value>>>>,
) -> (ChildAgent, mpsc::UnboundedReceiver<mpsc::Receiver<Update>>) {
    let held = room(name);
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch(name),
        "claude".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(autonomous_agent(reports, steering));
    let activities = driver.subscribe_unprompted().unwrap();
    driver.handshake(&ada, client_transport()).await.unwrap();
    let mut updates = driver
        .prompt("run the build".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert_eq!(turn_end(&mut updates).await, "end_turn");
    (driver, activities)
}

async fn next_activity(
    activities: &mut mpsc::UnboundedReceiver<mpsc::Receiver<Update>>,
) -> mpsc::Receiver<Update> {
    tokio::time::timeout(Duration::from_secs(5), activities.recv())
        .await
        .expect("no activity opened")
        .expect("the driver stopped handing over activities")
}

/// What an agent says after its turn is over reaches the room as an
/// activity of its own, ended by the usage report that closes the cycle.
#[tokio::test(flavor = "multi_thread")]
async fn what_the_agent_says_between_turns_is_its_own_activity() {
    let (driver, mut activities) = autonomous_driver("between-turns", true, None).await;
    let mut report = next_activity(&mut activities).await;
    assert!(matches!(next(&mut report).await, Update::Delta { text, .. } if text == "the build"));
    assert!(matches!(next(&mut report).await, Update::Delta { text, .. } if text == " passed"));
    assert!(
        matches!(next(&mut report).await, Update::Message { text, kind, .. }
            if text == "the build passed" && kind == MessageKind::Agent)
    );
    assert!(
        matches!(next(&mut report).await, Update::Turn { stop_reason, .. } if stop_reason == "end_turn")
    );
    assert!(report.recv().await.is_none());
    assert!(
        lock(&driver.live.history)
            .iter()
            .any(|fact| fact.contains("the build passed"))
    );

    // The next prompt is a turn of its own again.
    let mut updates = driver
        .prompt("anything else?".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert!(
        matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "hello again")
    );
    assert_eq!(turn_end(&mut updates).await, "end_turn");
}

/// A harness that never closes its report is closed by the operator's
/// next message, before that message's turn begins; and Stop ends one.
#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_or_a_stop_ends_work_the_agent_started_itself() {
    let (driver, mut activities) = autonomous_driver("unclosed-report", false, None).await;
    let mut report = next_activity(&mut activities).await;
    assert!(matches!(next(&mut report).await, Update::Delta { text, .. } if text == "the build"));
    assert!(matches!(next(&mut report).await, Update::Delta { text, .. } if text == " passed"));
    let mut updates = driver
        .prompt("and now?".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert!(
        matches!(next(&mut report).await, Update::Message { text, .. } if text == "the build passed")
    );
    assert!(
        matches!(next(&mut report).await, Update::Turn { stop_reason, .. } if stop_reason == "end_turn")
    );
    assert!(report.recv().await.is_none());
    assert!(
        matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "hello again")
    );
    assert_eq!(turn_end(&mut updates).await, "end_turn");

    let (driver, mut activities) = autonomous_driver("stopped-report", false, None).await;
    let mut report = next_activity(&mut activities).await;
    // Both pieces are in before the stop, so nothing the agent already
    // said can land after it.
    assert!(matches!(next(&mut report).await, Update::Delta { text, .. } if text == "the build"));
    assert!(matches!(next(&mut report).await, Update::Delta { text, .. } if text == " passed"));
    driver.cancel();
    // What it had said stays said: its words were already on screen.
    assert!(
        matches!(next(&mut report).await, Update::Message { text, .. } if text == "the build passed")
    );
    assert!(
        matches!(next(&mut report).await, Update::Turn { stop_reason, .. } if stop_reason == "aborted")
    );
    assert!(report.recv().await.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_autonomous_note_uses_non_starting_steering_and_drops_a_refusal() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (driver, mut activities) =
        autonomous_driver("quiet-note", false, Some(requests.clone())).await;
    let mut report = next_activity(&mut activities).await;
    assert!(matches!(next(&mut report).await, Update::Delta { .. }));
    driver.note_unprompted("This turn is quiet.".into()).await;
    let requests = lock(&requests);
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["_meta"]["steering"]["idleBehavior"],
        "promptRequired"
    );
    assert_eq!(requests[0]["prompt"][0]["text"], "This turn is quiet.");
    assert_eq!(
        driver.take_unconsumed(),
        [] as [(String, Vec<Attachment>); 0]
    );
    assert!(
        !lock(&driver.live.history)
            .iter()
            .any(|fact| fact.contains("This turn is quiet."))
    );
}

/// An agent that resumes its session, saying a line while it does, as Codex
/// warns that the session was recorded under another model.
fn resuming_agent() -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let running = agent_client_protocol::Agent
            .builder()
            .name("resuming")
            .on_receive_request(
                async |request: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version).agent_capabilities(
                            AgentCapabilities::new().session_capabilities(
                                acp::SessionCapabilities::new()
                                    .resume(acp::SessionResumeCapabilities::new()),
                            ),
                        ),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: acp::ResumeSessionRequest,
                       responder: Responder<acp::ResumeSessionResponse>,
                       cx: ConnectionTo<Client>| {
                    cx.send_notification(SessionNotification::new(
                        request.session_id,
                        SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(
                                "Warning: This session was recorded with model `gpt-6-luna` but is resuming with `gpt-6.2-sol`.",
                            ),
                        ))),
                    ))?;
                    responder.respond(acp::ResumeSessionResponse::new())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the resuming agent ended: {error}");
        }
    }
}

/// Codex on the desk, 2026-11-21 QA: each restart, its resume warning opened
/// an activity of the agent's own that Codex never reports ended, so the
/// teammate showed as working and its inbox waited five minutes, until the
/// quiet bound closed it. What an agent says while its session reopens is no
/// work it took up: no activity opens.
#[tokio::test(flavor = "multi_thread")]
async fn what_an_agent_says_while_its_session_resumes_opens_no_activity() {
    let held = room("resume-chatter");
    let ada = persona(
        &scratch_cwd(),
        vec![crate::contract::SessionCheckpoint {
            backend_id: "codex".into(),
            session_id: "recorded".into(),
        }],
    );
    let driver = ChildAgent::new(
        scratch("resume-chatter"),
        "codex".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(resuming_agent());
    let mut activities = driver.subscribe_unprompted().unwrap();
    driver.handshake(&ada, client_transport()).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(500), activities.recv())
            .await
            .is_err(),
        "the resume's warning opened an activity"
    );
    assert!(!driver.live.replaying.load(Ordering::SeqCst));
}
