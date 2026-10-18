use super::*;

/// What a steering agent was handed mid-turn, and how often it was told
/// to stop.
#[derive(Clone, Default)]
struct Steered {
    requests: Arc<Mutex<Vec<Value>>>,
    cancels: Arc<AtomicUsize>,
}

/// An agent that offers the steering extension and holds its turn open
/// until a line is steered into it, then answers the line with `outcome`
/// and ends the turn. `None` answers that it has no such method, which is
/// an agent that advertised more than it serves; `"silent"` never answers
/// and ends the turn only when told to stop.
fn steering_agent(
    steered: Steered,
    outcome: Option<&'static str>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let ends = Arc::new(tokio::sync::Notify::new());
        let ending = ends.clone();
        let stopping = ends.clone();
        let requests = steered.requests.clone();
        let cancels = steered.cancels.clone();
        let stopped = steered.cancels.clone();
        let mut meta = acp::Meta::new();
        meta.insert(
            "steering".to_string(),
            serde_json::json!({ "supported": true }),
        );
        let running = agent_client_protocol::Agent
            .builder()
            .name("steering")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version).meta(meta.clone()),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: NewSessionRequest,
                       responder: Responder<NewSessionResponse>,
                       _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("steered-session")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |_request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let ends = ends.clone();
                    let stopped = stopped.clone();
                    async move {
                        cx.spawn(async move {
                            ends.notified().await;
                            responder.respond(PromptResponse::new(
                                if stopped.load(Ordering::SeqCst) > 0 {
                                    StopReason::Cancelled
                                } else {
                                    StopReason::EndTurn
                                },
                            ))
                        })?;
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: agent_client_protocol::UntypedMessage,
                      responder: Responder<Value>,
                      cx: ConnectionTo<Client>| {
                    let requests = requests.clone();
                    let ending = ending.clone();
                    async move {
                        assert_eq!(request.method, STEERING_METHOD);
                        requests.lock().unwrap().push(request.params);
                        if outcome == Some("silent") {
                            return cx.spawn(async move {
                                std::future::pending::<()>().await;
                                responder.respond(Value::Null)
                            });
                        }
                        ending.notify_one();
                        match outcome {
                            Some(outcome) => {
                                responder.respond(serde_json::json!({ "outcome": outcome }))
                            }
                            None => responder.respond_with_error(
                                agent_client_protocol::Error::method_not_found(),
                            ),
                        }
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                move |_notification: CancelNotification, _cx| {
                    let cancels = cancels.clone();
                    let stopping = stopping.clone();
                    async move {
                        cancels.fetch_add(1, Ordering::SeqCst);
                        stopping.notify_one();
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the steering agent ended: {error}");
        }
    }
}

/// Steers a line into the driver's running turn, which is open once its
/// prompt is on the wire rather than when `prompt` returns.
async fn steer_into(driver: &ChildAgent, text: &str) {
    for _ in 0..200 {
        if driver.steer(text.to_string(), Vec::new()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the running turn never let a line in");
}

/// An agent that offers steering hears a line sent mid-turn inside that
/// turn, and asked not to start a turn of its own if it has gone idle.
/// Nothing comes back to be said again, and the line joins the history
/// a fresh session is briefed with.
#[tokio::test(flavor = "multi_thread")]
async fn a_line_sent_mid_turn_is_steered_into_the_running_turn() {
    let held = room("steer-room");
    let steered = Steered::default();
    let agent = steering_agent(steered.clone(), Some("injected"));
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch("steer"),
        "cursor".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);

    let info = driver.handshake(&ada, client_transport()).await.unwrap();
    assert!(info.capabilities.active_input);
    let mut updates = driver
        .prompt(
            "check the south pier".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;
    steer_into(&driver, "actually, the north pier").await;
    assert_eq!(turn_end(&mut updates).await, "end_turn");

    let requests = steered.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["sessionId"], "steered-session");
    assert_eq!(requests[0]["prompt"][0]["text"], "actually, the north pier");
    assert_eq!(
        requests[0]["_meta"]["steering"]["idleBehavior"],
        "promptRequired"
    );
    assert!(driver.take_unconsumed().is_empty());
    assert!(
        lock(&driver.live.history)
            .iter()
            .any(|fact| fact.starts_with("Operator: actually, the north pier"))
    );
    // Between turns there is nothing to steer into; the session queues.
    assert!(!driver.steer("and the east one".to_string(), Vec::new()));
}

/// A line the agent did not take comes back to the session once the turn
/// is over, to be said as a prompt of its own: when the agent had gone
/// idle, when it started a turn anyway (which is stopped, because nothing
/// it said there would reach the tape), and when it advertised the method
/// without serving it (which also ends steering for that agent).
#[tokio::test(flavor = "multi_thread")]
async fn a_line_the_agent_did_not_take_comes_back_to_be_said_again() {
    for outcome in [Some("promptRequired"), Some("startedNewTurn"), None] {
        let held = room("refused-steer-room");
        let steered = Steered::default();
        let agent = steering_agent(steered.clone(), outcome);
        let ada = persona(&scratch_cwd(), Vec::new());
        let driver = ChildAgent::new(
            scratch("refused-steer"),
            "cursor".to_string(),
            "briefing".to_string(),
            TeammateTools::new(&held, "ada"),
        );
        tokio::spawn(agent);
        driver.handshake(&ada, client_transport()).await.unwrap();

        let mut updates = driver
            .prompt(
                "check the south pier".to_string(),
                Vec::new(),
                Reach::Workspace,
            )
            .await;
        steer_into(&driver, "actually, the north pier").await;
        assert_eq!(turn_end(&mut updates).await, "end_turn", "{outcome:?}");
        let back: Vec<String> = driver
            .take_unconsumed()
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        assert_eq!(back, ["actually, the north pier"], "{outcome:?}");
        // The stop is sent before the turn ends and read a beat later.
        let stops = usize::from(outcome == Some("startedNewTurn"));
        for _ in 0..100 {
            if steered.cancels.load(Ordering::SeqCst) >= stops {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(steered.cancels.load(Ordering::SeqCst), stops, "{outcome:?}");
        let unserved = outcome.is_none();
        assert_eq!(driver.live.steerable.load(Ordering::SeqCst), !unserved);
        assert_eq!(
            lock(&driver.live.session).info.capabilities.active_input,
            !unserved
        );
    }
}

/// A reply cut off by a steered line and the reply to that line are two
/// messages on the tape when the agent names them apart, and one when it
/// sends no ids, which is how every agent streamed before.
#[tokio::test]
async fn a_new_message_id_from_the_agent_starts_a_new_message() {
    for (ids, expected) in [
        (
            [Some("m1"), Some("m2")],
            vec![
                "I'll check the south".to_string(),
                "North pier it is.".to_string(),
            ],
        ),
        (
            [None, None],
            vec!["I'll check the southNorth pier it is.".to_string()],
        ),
    ] {
        let live = Live::default();
        let (sender, mut updates) = mpsc::channel(16);
        *lock(&live.updates) = Some(sender);
        for (text, id) in ["I'll check the south", "North pier it is."]
            .into_iter()
            .zip(ids)
        {
            let id = id.map(acp::MessageId::new);
            live.pen()
                .chunk(MessageKind::Agent, text, id.as_ref())
                .await;
        }
        live.flush().await;
        lock(&live.updates).take();
        let mut messages = Vec::new();
        while let Some(update) = updates.recv().await {
            if let Update::Message { text, .. } = update {
                messages.push(text);
            }
        }
        assert_eq!(messages, expected);
    }
}

/// Stop ends a turn at once even while the agent sits on a steered line
/// without answering it: the line is dropped with the turn, and nothing
/// waits out the reply's timeout first.
#[tokio::test(flavor = "multi_thread")]
async fn stop_does_not_wait_on_a_steered_line_the_agent_never_answers() {
    let held = room("silent-steer-room");
    let steered = Steered::default();
    let agent = steering_agent(steered.clone(), Some("silent"));
    let ada = persona(&scratch_cwd(), Vec::new());
    let driver = ChildAgent::new(
        scratch("silent-steer"),
        "cursor".to_string(),
        "briefing".to_string(),
        TeammateTools::new(&held, "ada"),
    );
    tokio::spawn(agent);
    driver.handshake(&ada, client_transport()).await.unwrap();

    let mut updates = driver
        .prompt(
            "check the south pier".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;
    steer_into(&driver, "actually, the north pier").await;
    for _ in 0..200 {
        if !steered.requests.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(steered.requests.lock().unwrap().len(), 1);

    driver.cancel();
    // `next` gives each update five seconds, well short of STEER_REPLY.
    assert_eq!(turn_end(&mut updates).await, "cancelled");
    assert!(driver.take_unconsumed().is_empty());
}
