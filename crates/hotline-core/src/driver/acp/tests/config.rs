use super::*;

fn select_config(
    id: &str,
    name: &str,
    category: acp::SessionConfigOptionCategory,
    current: &str,
    options: &[(&str, &str)],
) -> acp::SessionConfigOption {
    acp::SessionConfigOption::select(
        id.to_string(),
        name.to_string(),
        current.to_string(),
        options
            .iter()
            .map(|(value, label)| {
                acp::SessionConfigSelectOption::new((*value).to_string(), (*label).to_string())
            })
            .collect::<Vec<_>>(),
    )
    .category(Some(category))
}

#[test]
fn disposition_separates_runtime_mode_from_effort_and_hides_other_configs() {
    let configs = vec![
        select_config(
            "model-choice",
            "Model",
            acp::SessionConfigOptionCategory::Model,
            "model-2",
            &[("model-1", "Model One"), ("model-2", "Model Two")],
        ),
        select_config(
            "permission-mode",
            "Permission mode",
            acp::SessionConfigOptionCategory::Mode,
            "agent",
            &[("ask", "Ask"), ("agent", "Agent")],
        ),
        select_config(
            "reasoning",
            "Reasoning",
            acp::SessionConfigOptionCategory::ThoughtLevel,
            "high",
            &[("low", "Low"), ("high", "High")],
        ),
        select_config(
            "style",
            "Style",
            acp::SessionConfigOptionCategory::Other("style".to_string()),
            "concise",
            &[("concise", "Concise")],
        ),
    ];
    let generic = Disposition::of(None, Some(&configs));
    assert_eq!(generic.current_model_id.as_deref(), Some("model-2"));
    assert_eq!(generic.model_config.as_deref(), Some("model-choice"));
    assert_eq!(generic.current_mode_id.as_deref(), Some("agent"));
    assert_eq!(generic.mode_config.as_deref(), Some("permission-mode"));
    assert_eq!(generic.configs.len(), 1);
    assert_eq!(generic.configs[0].id, "reasoning");
    assert_eq!(
        generic.configs[0].category,
        Some(crate::contract::SessionConfigCategory::Effort)
    );

    let dedicated_modes = SessionModeState::new(
        acp::SessionModeId::new("ask"),
        vec![
            SessionMode::new(acp::SessionModeId::new("ask"), "Ask"),
            SessionMode::new(acp::SessionModeId::new("agent"), "Agent"),
        ],
    );
    let dedicated = Disposition::of(Some(&dedicated_modes), Some(&configs));
    assert_eq!(dedicated.current_mode_id.as_deref(), Some("ask"));
    assert!(dedicated.mode_config.is_none());
    assert_eq!(dedicated.configs.len(), 1);
    assert_eq!(dedicated.configs[0].id, "reasoning");
}

#[tokio::test]
async fn config_notifications_update_model_and_effort_without_replacing_mode() {
    let live = Live::default();
    let dedicated_modes = SessionModeState::new(
        acp::SessionModeId::new("ask"),
        vec![
            SessionMode::new(acp::SessionModeId::new("ask"), "Ask"),
            SessionMode::new(acp::SessionModeId::new("agent"), "Agent"),
        ],
    );
    {
        let mut session = lock(&live.session);
        let disposition = Disposition::of(Some(&dedicated_modes), None);
        session.dedicated_modes = true;
        session.info.modes = disposition.modes;
        session.info.current_mode_id = disposition.current_mode_id;
    }
    let update = SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(vec![
        select_config(
            "model-choice",
            "Model",
            acp::SessionConfigOptionCategory::Model,
            "model-2",
            &[("model-1", "Model One"), ("model-2", "Model Two")],
        ),
        select_config(
            "permission-mode",
            "Permission mode",
            acp::SessionConfigOptionCategory::Mode,
            "agent",
            &[("ask", "Ask"), ("agent", "Agent")],
        ),
        select_config(
            "reasoning",
            "Reasoning",
            acp::SessionConfigOptionCategory::ThoughtLevel,
            "high",
            &[("low", "Low"), ("high", "High")],
        ),
    ]));
    translate(&live, update).await;

    {
        let session = lock(&live.session);
        assert_eq!(session.info.current_model_id, "model-2");
        assert_eq!(session.info.current_mode_id.as_deref(), Some("ask"));
        assert_eq!(
            session
                .info
                .modes
                .iter()
                .map(|mode| mode.id.as_str())
                .collect::<Vec<_>>(),
            ["ask", "agent"]
        );
        assert_eq!(session.info.configs.len(), 1);
        assert_eq!(session.info.configs[0].current_id.as_deref(), Some("high"));
    }

    // A later full-set notification can remove model and config-based
    // mode controls. Those old ids must disappear rather than remaining
    // callable through a stale picker.
    let removal =
        SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(vec![select_config(
            "reasoning",
            "Reasoning",
            acp::SessionConfigOptionCategory::ThoughtLevel,
            "low",
            &[("low", "Low"), ("high", "High")],
        )]));
    translate(&live, removal).await;
    let session = lock(&live.session);
    assert!(session.model_config.is_none());
    assert_eq!(session.info.models, [] as [crate::driver::ConfigChoice; 0]);
    assert_eq!(session.info.current_model_id, "");
    assert_eq!(session.info.current_mode_id.as_deref(), Some("ask"));
    assert_eq!(session.info.configs.len(), 1);
}

#[tokio::test]
async fn config_notifications_remove_config_mode_without_stale_picker() {
    let live = Live::default();
    let initial =
        SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(vec![select_config(
            "runtime-mode",
            "Runtime mode",
            acp::SessionConfigOptionCategory::Mode,
            "build",
            [("plan", "Plan"), ("build", "Build")].as_slice(),
        )]));
    translate(&live, initial).await;

    {
        let session = lock(&live.session);
        assert_eq!(session.mode_config.as_deref(), Some("runtime-mode"));
        assert_eq!(session.info.current_mode_id.as_deref(), Some("build"));
        assert_eq!(session.info.modes.len(), 2);
    }

    // The ACP notification is a full replacement. Removing the mode
    // option must make the old wire command and UI picker unavailable.
    let removal =
        SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(vec![select_config(
            "reasoning",
            "Reasoning",
            acp::SessionConfigOptionCategory::ThoughtLevel,
            "high",
            [("low", "Low"), ("high", "High")].as_slice(),
        )]));
    translate(&live, removal).await;

    let session = lock(&live.session);
    assert!(session.mode_config.is_none());
    assert_eq!(session.info.modes, [] as [crate::driver::ConfigChoice; 0]);
    assert!(session.info.current_mode_id.is_none());
    assert!(session.info.mode_label.is_none());
    assert_eq!(session.info.configs.len(), 1);
}

/// `session.set_model` and `session.set_mode` are two commands, and the
/// wire answers each socket's on its own task. Both read the connection
/// and the session; taking those two locks in opposite orders wedges both
/// threads for good — no timeout, no error, and every later call on this
/// driver queued behind them.
#[tokio::test]
async fn setting_a_config_and_a_mode_at_once_does_not_wedge_the_driver() {
    let held = room("lock-order-room");
    let driver = Arc::new(ChildAgent::new(
        scratch("lock-order"),
        "cursor".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    ));
    let finished = Arc::new(AtomicUsize::new(0));
    const ROUNDS: usize = 50_000;

    for worker in 0..2 {
        let driver = driver.clone();
        let finished = finished.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime for this worker");
            runtime.block_on(async {
                for _ in 0..ROUNDS {
                    let _ = match worker {
                        0 => driver.set_config("config", "value").await,
                        _ => driver.set_mode("mode").await,
                    };
                }
            });
            finished.fetch_add(1, Ordering::SeqCst);
        });
    }

    for _ in 0..600 {
        if finished.load(Ordering::SeqCst) == 2 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the driver stopped answering: two callers are holding each other's lock");
}

/// The pickers a scripted agent opens with: Model (Opus, current, or
/// Sonnet), Effort (Low, Medium, current, or High), and two modes, Default
/// (current) and Plan.
fn offered_configs(model: &str, effort: &str) -> Vec<acp::SessionConfigOption> {
    vec![
        select_config(
            "model-choice",
            "Model",
            acp::SessionConfigOptionCategory::Model,
            model,
            &[("opus", "Opus"), ("sonnet", "Sonnet")],
        ),
        select_config(
            "reasoning",
            "Effort",
            acp::SessionConfigOptionCategory::ThoughtLevel,
            effort,
            &[("low", "Low"), ("medium", "Medium"), ("high", "High")],
        ),
    ]
}

/// An agent that opens a session with [`offered_configs`] and dedicated
/// modes, and writes down every setting and mode it is asked for.
fn agent_that_records_settings(
    asked: Arc<Mutex<Vec<String>>>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let configs = Arc::new(Mutex::new(("opus".to_string(), "medium".to_string())));
        let set_configs = configs.clone();
        let set_asked = asked.clone();
        let mode_asked = asked.clone();
        let prompt_asked = asked;
        let running = agent_client_protocol::Agent
            .builder()
            .name("recording")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    responder.respond(InitializeResponse::new(request.protocol_version))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |_request: NewSessionRequest,
                      responder: Responder<NewSessionResponse>,
                      _cx| {
                    let configs = configs.clone();
                    async move {
                        let (model, effort) = configs.lock().unwrap().clone();
                        responder.respond(
                            NewSessionResponse::new(SessionId::new("recorded"))
                                .modes(SessionModeState::new(
                                    acp::SessionModeId::new("default"),
                                    vec![
                                        SessionMode::new(
                                            acp::SessionModeId::new("default"),
                                            "Default",
                                        ),
                                        SessionMode::new(acp::SessionModeId::new("plan"), "Plan"),
                                    ],
                                ))
                                .config_options(offered_configs(&model, &effort)),
                        )
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: acp::SetSessionConfigOptionRequest,
                      responder: Responder<acp::SetSessionConfigOptionResponse>,
                      _cx| {
                    let configs = set_configs.clone();
                    let asked = set_asked.clone();
                    async move {
                        let acp::SessionConfigOptionValue::ValueId { value } = &request.value
                        else {
                            panic!("a select is set by its value's id");
                        };
                        let value = value.0.to_string();
                        asked
                            .lock()
                            .unwrap()
                            .push(format!("{} = {value}", request.config_id.0));
                        let (model, effort) = {
                            let mut current = configs.lock().unwrap();
                            match request.config_id.0.as_ref() {
                                "model-choice" => current.0 = value,
                                "reasoning" => current.1 = value,
                                _ => {}
                            }
                            current.clone()
                        };
                        responder.respond(acp::SetSessionConfigOptionResponse::new(
                            offered_configs(&model, &effort),
                        ))
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: acp::SetSessionModeRequest,
                      responder: Responder<acp::SetSessionModeResponse>,
                      _cx| {
                    let asked = mode_asked.clone();
                    async move {
                        asked
                            .lock()
                            .unwrap()
                            .push(format!("mode = {}", request.mode_id.0));
                        responder.respond(acp::SetSessionModeResponse::new())
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |_request: acp::PromptRequest, responder: Responder<PromptResponse>, _cx| {
                    let asked = prompt_asked.clone();
                    async move {
                        asked.lock().unwrap().push("prompt".to_string());
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the recording agent ended: {error}");
        }
    }
}

/// What a teammate chose while it was not running is put on the session
/// that opens next — mode, model, then effort, which a model can change —
/// and a choice the agent no longer offers is not sent at all. What the
/// agent offered as it opened is kept for the agent, on disk, with the
/// values it opened on, before any of the teammate's choices.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_puts_the_teammates_choices_back_and_keeps_what_the_agent_offered() {
    let held = room("choices-room");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let agent = agent_that_records_settings(asked.clone());
    let root = scratch("choices");
    let driver = ChildAgent::new(
        root.clone(),
        "claude-acp".to_string(),
        "you are Ada".to_string(),
        TeammateTools::new(&held, "choices"),
    );
    tokio::spawn(agent);

    let mut ada = persona(&scratch_cwd(), Vec::new());
    ada.id = "choices".to_string();
    ada.backend_id = "claude-acp".to_string();
    ada.mode_id = Some("plan".to_string());
    ada.model_id = Some("sonnet".to_string());
    ada.effort_id = Some("high".to_string());
    // A setting the agent no longer offers: dropped without a word.
    ada.config_choices =
        std::collections::BTreeMap::from([("verbosity".to_string(), "terse".to_string())]);
    let started = driver.handshake(&ada, client_transport()).await.unwrap();

    assert_eq!(
        *asked.lock().unwrap(),
        ["mode = plan", "model-choice = sonnet", "reasoning = high"]
    );
    assert_eq!(started.current_model_id, "sonnet");
    assert_eq!(started.current_mode_id.as_deref(), Some("plan"));

    // Read from the file, as a desk opened again reads it.
    let offer = super::super::models::offer(&root, "claude-acp").expect("the offer is kept");
    assert_eq!(
        offer
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["opus", "sonnet"]
    );
    assert_eq!(offer.current_model_id.as_deref(), Some("opus"));
    assert_eq!(
        offer
            .modes
            .iter()
            .map(|mode| mode.id.as_str())
            .collect::<Vec<_>>(),
        ["default", "plan"]
    );
    assert_eq!(offer.current_mode_id.as_deref(), Some("default"));
    assert_eq!(offer.configs.len(), 1);
    assert_eq!(offer.configs[0].id, "reasoning");
    assert_eq!(offer.configs[0].current_id.as_deref(), Some("medium"));
}

/// Nothing chosen, nothing sent: a teammate that never chose opens on the
/// agent's own defaults, and one whose choices are already the agent's sends
/// nothing either.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_with_no_choices_or_the_agents_own_sends_nothing() {
    let held = room("no-choices-room");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let agent = agent_that_records_settings(asked.clone());
    let driver = ChildAgent::new(
        scratch("no-choices"),
        "claude-acp".to_string(),
        "you are Ada".to_string(),
        TeammateTools::new(&held, "no-choices"),
    );
    tokio::spawn(agent);
    let mut ada = persona(&scratch_cwd(), Vec::new());
    ada.id = "no-choices".to_string();
    ada.backend_id = "claude-acp".to_string();
    ada.model_id = Some("opus".to_string());
    ada.effort_id = Some("medium".to_string());
    ada.mode_id = Some("gone".to_string());
    driver.handshake(&ada, client_transport()).await.unwrap();
    assert!(
        asked.lock().unwrap().is_empty(),
        "{:?}",
        asked.lock().unwrap()
    );
}

/// A mode chosen while the teammate rested reaches its next session as
/// `session/set_mode`, answered before the first prompt is sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_mode_chosen_at_rest_is_set_before_the_first_prompt() {
    let held = room("rest-mode-room");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let agent = agent_that_records_settings(asked.clone());
    let driver = ChildAgent::new(
        scratch("rest-mode"),
        "claude-acp".to_string(),
        "you are Ada".to_string(),
        TeammateTools::new(&held, "rest-mode"),
    );
    tokio::spawn(agent);
    let mut ada = persona(&scratch_cwd(), Vec::new());
    ada.id = "rest-mode".to_string();
    ada.backend_id = "claude-acp".to_string();
    ada.mode_id = Some("plan".to_string());
    driver.handshake(&ada, client_transport()).await.unwrap();
    let mut updates = driver
        .prompt("hello".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert_eq!(turn_end(&mut updates).await, "end_turn");
    assert_eq!(*asked.lock().unwrap(), ["mode = plan", "prompt"]);
}
