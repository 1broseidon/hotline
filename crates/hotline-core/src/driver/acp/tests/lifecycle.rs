//! The connection's life: a load that outlasts its bound, the end of one turn
//! meeting the start of the next, a child that dies idle, and a connection
//! that never got a session.

use super::*;

/// How the fixture agent misbehaves.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// `session/load` never answers.
    StallLoad,
    /// The first prompt is answered without a word, so nothing is left to
    /// write down when its turn ends.
    SilentFirst,
    /// `session/new` is refused.
    RefuseSession,
}

/// An agent whose every prompt waits for a permit on `gate`, then says
/// `reply-N` and ends the turn.
fn gated_agent(
    fault: Fault,
    gate: Arc<tokio::sync::Semaphore>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let count = Arc::new(AtomicUsize::new(0));
        let _ = agent_client_protocol::Agent
            .builder()
            .name("gated")
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
                async move |_request: LoadSessionRequest,
                            responder: Responder<LoadSessionResponse>,
                            _cx| {
                    if fault == Fault::StallLoad {
                        std::future::pending::<()>().await;
                    }
                    responder.respond(LoadSessionResponse::new())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: NewSessionRequest,
                            responder: Responder<NewSessionResponse>,
                            _cx| {
                    if fault == Fault::RefuseSession {
                        return responder.respond_with_internal_error("no session for you");
                    }
                    responder.respond(NewSessionResponse::new(SessionId::new("gated")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: PromptRequest,
                      responder: Responder<PromptResponse>,
                      cx: ConnectionTo<Client>| {
                    let gate = gate.clone();
                    let count = count.clone();
                    async move {
                        let later = cx.clone();
                        cx.spawn(async move {
                            gate.acquire().await.unwrap().forget();
                            let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                            if fault != Fault::SilentFirst || n > 1 {
                                later.send_notification(SessionNotification::new(
                                    request.session_id,
                                    SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                        ContentBlock::Text(TextContent::new(format!("reply-{n}"))),
                                    )),
                                ))?;
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
    }
}

fn driver(name: &str) -> (ChildAgent, Arc<crate::session::Room>) {
    let held = room(name);
    let driver = ChildAgent::new(
        scratch(name),
        "cursor".into(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    );
    (driver, held)
}

fn checkpointed() -> Persona {
    persona(
        &scratch_cwd(),
        vec![SessionCheckpoint {
            backend_id: "cursor".into(),
            session_id: "old".into(),
        }],
    )
}

async fn said(updates: &mut mpsc::Receiver<Update>) -> String {
    let mut text = String::new();
    loop {
        match next(updates).await {
            Update::Message { text: more, .. } => text.push_str(&more),
            Update::Turn { .. } => return text,
            _ => {}
        }
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}");
}

/// A `session/load` that outlasts the start bound is dropped mid-replay. The
/// replay it began must end with it, or the restart that follows hears
/// nothing the agent says.
#[tokio::test(flavor = "multi_thread")]
async fn a_load_that_stalls_past_the_bound_does_not_leave_the_driver_deaf() {
    let (driver, _held) = driver("stalled-load");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let stalled = tokio::spawn(gated_agent(Fault::StallLoad, gate.clone()));
    let ada = checkpointed();
    let bounded = tokio::time::timeout(
        Duration::from_millis(300),
        driver.handshake(&ada, client_transport()),
    )
    .await;
    assert!(bounded.is_err(), "the load was meant to outlast the bound");
    assert!(
        !driver.live.replaying.load(Ordering::SeqCst),
        "the replay outlived the load that began it"
    );

    // The restart: the old child is abandoned and a new one opens a session.
    let fresh = driver.abandon_failed_session().await.unwrap();
    stalled.abort();
    tokio::spawn(gated_agent(Fault::None, gate.clone()));
    driver.handshake(&fresh, client_transport()).await.unwrap();
    gate.add_permits(1);
    let mut updates = driver
        .prompt("hello".into(), Vec::new(), Reach::Workspace)
        .await;
    assert_eq!(said(&mut updates).await, "reply-1");
}

/// The prompt task finishes after it has sent `Turn`, and by then the session
/// may have begun the next prompt. What the old task clears must be its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_finished_prompt_does_not_clear_the_next_prompts_sink() {
    let (driver, _held) = driver("sink");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    tokio::spawn(gated_agent(Fault::SilentFirst, gate.clone()));
    driver
        .handshake(&persona(&scratch_cwd(), Vec::new()), client_transport())
        .await
        .unwrap();
    let quiet = Arc::strong_count(&driver.live);

    let mut first = driver
        .prompt("one".into(), Vec::new(), Reach::Workspace)
        .await;
    // Wedge the first turn's channel, so its task is held at the `Turn` it
    // sends, after it has let go of the sink and before it finishes.
    let sink = lock(&driver.live.updates).clone().expect("turn one runs");
    for _ in 0..UPDATE_DEPTH {
        sink.try_send(Update::Notice {
            level: NoticeLevel::Info,
            text: "full".into(),
        })
        .unwrap();
    }
    gate.add_permits(1);
    until("turn one never reached its end", || {
        lock(&driver.live.updates).is_none()
    })
    .await;

    // The next prompt starts in that gap.
    let mut second = driver
        .prompt("two".into(), Vec::new(), Reach::Workspace)
        .await;
    loop {
        if matches!(first.recv().await, Some(Update::Turn { .. })) {
            break;
        }
    }
    // Turn one's task is gone; only turn two's remains.
    until("turn one's task never finished", || {
        Arc::strong_count(&driver.live) == quiet + 1
    })
    .await;
    gate.add_permits(1);
    assert_eq!(said(&mut second).await, "reply-2");
}

/// A child that exits between turns is found out when it exits, not by the
/// prompt that fails against it.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_that_dies_idle_marks_the_driver_failed() {
    let (driver, _held) = driver("dies-idle");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let agent = tokio::spawn(gated_agent(Fault::None, gate));
    driver
        .handshake(&persona(&scratch_cwd(), Vec::new()), client_transport())
        .await
        .unwrap();
    assert!(driver.checkpoint_valid());
    agent.abort();
    until("the driver never noticed the child had gone", || {
        !driver.checkpoint_valid()
    })
    .await;
    assert!(driver.live.needs_restart());
}

/// A connection whose session never opened restarts on the next prompt; it
/// does not answer "no open session" for good.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_that_never_got_a_session_restarts() {
    let (driver, _held) = driver("sessionless");
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    tokio::spawn(gated_agent(Fault::RefuseSession, gate));
    let ada = persona(&scratch_cwd(), Vec::new());
    assert!(driver.handshake(&ada, client_transport()).await.is_err());
    assert!(!driver.checkpoint_valid(), "a refused session is a failure");
    assert!(driver.live.needs_restart());

    // Restarting needs a launch configuration; with none, the prompt says so,
    // which is how this shows it tried and did not stop at "no open session".
    *lock(&driver.persona) = None;
    let mut updates = driver
        .prompt("hello".into(), Vec::new(), Reach::Workspace)
        .await;
    let Update::Notice { text, .. } = next(&mut updates).await else {
        panic!("the prompt did not fail");
    };
    assert!(text.contains("launch configuration"), "{text}");
}
