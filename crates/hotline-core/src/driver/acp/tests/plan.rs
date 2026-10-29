use super::*;

/// What `formatUsageResponse` writes for an account with both windows.
const USAGE: &str = "## Usage\n\n> Claude max subscription usage\n\n### Limits\n\n**5-hour limit** — **38%** · Resets Oct 29, 9:00 PM CDT\n\n`████████░░░░░░░░░░░░`\n\n**Weekly · all models** — **61%** · Resets Nov 2, 9:00 AM CST\n\n`████████████░░░░░░░░`\n\n---\n\n### This session\n\n| Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 9s |";

/// An agent that answers `/usage` as Claude Code's adapter does: the
/// Markdown as agent message chunks, then the end of the turn, with no
/// model call behind it. Any other prompt is a turn it holds open until
/// `release` is notified, saying one word first.
fn usage_agent(
    prompted: Arc<Mutex<Vec<String>>>,
    release: Arc<tokio::sync::Notify>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let running = agent_client_protocol::Agent
            .builder()
            .name("claude-usage")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    responder.respond(InitializeResponse::new(request.protocol_version))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: NewSessionRequest,
                       responder: Responder<NewSessionResponse>,
                       _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("usage-session")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let prompted = prompted.clone();
                    let release = release.clone();
                    async move {
                        let texts: Vec<String> = request
                            .prompt
                            .iter()
                            .filter_map(|block| match block {
                                ContentBlock::Text(text) => Some(text.text.clone()),
                                _ => None,
                            })
                            .collect();
                        prompted.lock().unwrap().extend(texts.iter().cloned());
                        let session = request.session_id.clone();
                        let say = |text: &str| {
                            cx.send_notification(SessionNotification::new(
                                session.clone(),
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new(text)),
                                )),
                            ))
                        };
                        if texts == ["/usage"] {
                            let (head, tail) = USAGE.split_at(USAGE.find("**Weekly").unwrap());
                            say(head)?;
                            say(tail)?;
                            // The local command closes its cycle like any.
                            cx.send_notification(SessionNotification::new(
                                session.clone(),
                                SessionUpdate::UsageUpdate(
                                    acp::UsageUpdate::new(0, 200_000)
                                        .cost(acp::Cost::new(0.0, "USD")),
                                ),
                            ))?;
                            return responder.respond(PromptResponse::new(StopReason::EndTurn));
                        }
                        say("working")?;
                        cx.spawn(async move {
                            release.notified().await;
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
            eprintln!("the usage agent ended: {error}");
        }
    }
}

struct Probed {
    driver: ChildAgent,
    activities: mpsc::UnboundedReceiver<mpsc::Receiver<Update>>,
    prompted: Arc<Mutex<Vec<String>>>,
    release: Arc<tokio::sync::Notify>,
}

async fn claude_code(name: &str) -> Probed {
    let held = room(name);
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch(name),
        "claude-acp".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    let prompted = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    tokio::spawn(usage_agent(prompted.clone(), release.clone()));
    let activities = driver.subscribe_unprompted().unwrap();
    driver.handshake(&ada, client_transport()).await.unwrap();
    Probed {
        driver,
        activities,
        prompted,
        release,
    }
}

/// `/usage` is said between turns and its answer becomes the reading; none
/// of it opens an activity, reaches a turn, joins the history a fresh
/// session is briefed with, or spends the briefing the next prompt carries.
#[tokio::test(flavor = "multi_thread")]
async fn the_plan_is_read_with_usage_and_nothing_it_says_reaches_a_turn_or_the_tape() {
    let mut probed = claude_code("plan-probe").await;
    let plan = probed.driver.read_plan().await.expect("a reading");
    assert_eq!(
        plan.windows
            .iter()
            .map(|one| (one.kind.as_str(), one.used_percent))
            .collect::<Vec<_>>(),
        [("five_hour", Some(38.0)), ("seven_day", Some(61.0))]
    );
    assert!(plan.windows.iter().all(|one| one.resets_at.is_some()));
    assert_eq!(*lock(&probed.prompted), ["/usage"]);
    assert!(lock(&probed.driver.live.probe).is_none());

    // Nothing opened an activity of the agent's own.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(probed.activities.try_recv().is_err());
    assert!(lock(&probed.driver.live.history).is_empty());
    assert!(lock(&probed.driver.live.open).is_none());

    // The next prompt is the first thing said: it carries the briefing, and
    // its turn holds its own words and none of the reading's.
    let mut updates = probed
        .driver
        .prompt("hello".to_string(), Vec::new(), Reach::Workspace)
        .await;
    probed.release.notify_one();
    let mut said = Vec::new();
    loop {
        match next(&mut updates).await {
            Update::Turn { stop_reason, .. } => {
                assert_eq!(stop_reason, "end_turn");
                break;
            }
            Update::Delta { text, .. } | Update::Message { text, .. } => said.push(text),
            _ => {}
        }
    }
    assert_eq!(said, ["working", "working"]);
    assert_eq!(*lock(&probed.prompted), ["/usage", "briefing", "hello"]);
}

/// While a turn runs the plan is not read: `/usage` would be a second
/// prompt in the turn's stream. Once the turn is over it is.
#[tokio::test(flavor = "multi_thread")]
async fn the_plan_is_not_read_while_a_turn_runs() {
    let probed = claude_code("plan-busy").await;
    let mut updates = probed
        .driver
        .prompt("work".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert!(matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "working"));
    assert_eq!(probed.driver.read_plan().await, None);
    assert!(!lock(&probed.prompted).iter().any(|text| text == "/usage"));
    probed.release.notify_one();
    assert_eq!(turn_end(&mut updates).await, "end_turn");
    assert!(probed.driver.read_plan().await.is_some());
}

/// Only Claude Code answers `/usage` itself; any other agent is never sent
/// it.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_is_not_claude_code_is_never_sent_usage() {
    let held = room("plan-other");
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch("plan-other"),
        "gemini".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    let prompted = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(usage_agent(
        prompted.clone(),
        Arc::new(tokio::sync::Notify::new()),
    ));
    driver.handshake(&ada, client_transport()).await.unwrap();
    assert_eq!(driver.read_plan().await, None);
    assert!(lock(&prompted).is_empty());
}

/// A Codex teammate's plan is read from the session logs of the Codex home
/// it was launched with, and is read again after every turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_codex_teammates_plan_is_read_from_its_logs() {
    let held = room("plan-codex");
    let home = scratch("plan-codex-home");
    let day = home.join("sessions/2026/10/29");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(
        day.join("rollout-2026-10-29T08-00-00-a.jsonl"),
        r#"{"timestamp":"2026-10-29T08:00:01.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":2.0,"window_minutes":10080,"resets_at":1793774976},"secondary":null}}}"#,
    )
    .unwrap();
    let driver = ChildAgent::new(
        scratch("plan-codex"),
        "codex-acp".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    *lock(&driver.launch) = Some(registry::Launch {
        command: "codex-acp".to_string(),
        args: Vec::new(),
        env: vec![(
            "CODEX_HOME".to_string(),
            home.to_string_lossy().into_owned(),
        )],
    });
    assert!(driver.plan_after_turn());
    let plan = driver.read_plan().await.expect("a reading");
    assert_eq!(plan.windows.len(), 1);
    assert_eq!(plan.windows[0].kind, "seven_day");
    assert_eq!(plan.windows[0].resets_at, Some(1_793_774_976_000));
}
