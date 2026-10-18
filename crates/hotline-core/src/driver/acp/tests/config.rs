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
    assert!(session.info.models.is_empty());
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
    assert!(session.info.modes.is_empty());
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
