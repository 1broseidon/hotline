use super::*;
use crate::credentials::KEYCHAIN_LOCKED;
use crate::driver::failure::ACCOUNT_SIGNED_OUT;
use hotline_room::provider_auth::ChosenAccount;
use std::sync::atomic::AtomicUsize;

/// What the vault would hand a turn: `chosen` when the driver names an
/// account, `default` when it names none. Every ask is kept, so a call site
/// that forgot the account shows up as a `None` among the `Some`s.
#[derive(Default)]
struct Keys {
    default: HashMap<String, ProviderAuth>,
    chosen: HashMap<String, ProviderAuth>,
    default_account: Option<(String, bool)>,
    asked: Mutex<Vec<Option<ChosenAccount>>>,
    defaults_asked: AtomicUsize,
}

impl ProviderKeys for Keys {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        self.default.clone()
    }

    fn provider_auth_for(&self, account: Option<&ChosenAccount>) -> HashMap<String, ProviderAuth> {
        lock(&self.asked).push(account.cloned());
        match account {
            Some(_) => self.chosen.clone(),
            None => self.default.clone(),
        }
    }

    fn default_account(&self, _provider_id: &str) -> Option<(String, bool)> {
        self.defaults_asked.fetch_add(1, Ordering::SeqCst);
        self.default_account.clone()
    }
}

fn unavailable(why: &str) -> ProviderAuth {
    ProviderAuth::Unavailable(why.to_string())
}

fn keys(default: ProviderAuth, chosen: ProviderAuth) -> Keys {
    Keys {
        default: HashMap::from([("anthropic".to_string(), default)]),
        chosen: HashMap::from([("anthropic".to_string(), chosen)]),
        ..Keys::default()
    }
}

fn account_b() -> ChosenAccount {
    ChosenAccount {
        id: "b".to_string(),
        provider_id: "anthropic".to_string(),
    }
}

/// The teammate's tools hold the room weakly, so the test holds it.
struct Agent {
    driver: InProcess,
    _room: Arc<crate::session::Room>,
    _scratch: tempfile::TempDir,
}

async fn started(name: &str, keys: Arc<Keys>, account: Option<ChosenAccount>) -> Agent {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let room = crate::session::Room::new(crate::log::Log::open(root.join("room")), keys.clone());
    let persona = Persona {
        cwd: root.to_string_lossy().into_owned(),
        reach: Some(Reach::Workspace),
        model_id: Some("anthropic/claude-sonnet-4-5".to_string()),
        ..test_persona(&format!("rig-accounts-{name}"))
    };
    let driver = InProcess::new(
        keys,
        String::new(),
        Vec::new(),
        root.join("out"),
        TeammateTools::new(&room, persona.id.clone()),
    )
    .with_account(account);
    Driver::start(&driver, &persona).await.unwrap();
    Agent {
        driver,
        _room: room,
        _scratch: scratch,
    }
}

/// One turn to its end: how it stopped, and the notice it gave, if any.
async fn turn(driver: &InProcess) -> (String, Option<String>) {
    let mut updates = driver
        .prompt(
            "Is the crane free?".to_string(),
            Vec::new(),
            Reach::Workspace,
        )
        .await;
    let mut notice = None;
    while let Some(update) = updates.recv().await {
        match update {
            Update::Notice { text, .. } => notice = Some(text),
            Update::Turn { stop_reason, .. } => return (stop_reason, notice),
            _ => {}
        }
    }
    panic!("the turn never ended");
}

/// The notice a turn that cannot reach its provider ends with.
async fn failed_turn(driver: &InProcess) -> String {
    let (stop_reason, notice) = turn(driver).await;
    assert_eq!(stop_reason, "failed");
    notice.expect("a failed turn says why")
}

#[tokio::test]
async fn a_chosen_account_signs_only_its_providers_turns() {
    for account in [Some(account_b()), None] {
        let keys = Arc::new(Keys {
            default: HashMap::from([
                ("anthropic".to_string(), unavailable("anthropic's default")),
                ("openai".to_string(), unavailable("openai's default")),
            ]),
            chosen: HashMap::from([
                ("anthropic".to_string(), unavailable("account b")),
                ("openai".to_string(), unavailable("openai's default")),
            ]),
            ..Keys::default()
        });
        let agent = started("chosen-signs", keys.clone(), account.clone()).await;
        let driver = &agent.driver;
        assert!(driver.current_info().is_some());
        driver.set_config("effort", "").await.unwrap();
        let on_anthropic = failed_turn(driver).await;
        driver.set_model("openai/gpt-5.5").await.unwrap();
        let on_openai = failed_turn(driver).await;

        assert_eq!(*lock(&keys.asked), vec![account.clone(); 6]);
        let signed_by = if account.is_some() {
            "account b"
        } else {
            "anthropic's default"
        };
        assert!(on_anthropic.contains(signed_by), "{on_anthropic}");
        assert_eq!(
            on_anthropic.contains(r#""accountId":"b""#),
            account.is_some(),
            "{on_anthropic}"
        );
        assert!(on_openai.contains("openai's default"), "{on_openai}");
        assert!(!on_openai.contains("accountId"), "{on_openai}");
    }
}

#[tokio::test]
async fn a_turn_on_a_signed_out_account_fails_as_a_sign_in_and_names_it_by_id() {
    const DEFAULTS_WORDS: &str = "the default credential";
    let keys = Arc::new(keys(
        unavailable(DEFAULTS_WORDS),
        unavailable(ACCOUNT_SIGNED_OUT),
    ));
    let agent = started("signed-out", keys, Some(account_b())).await;
    let notice = failed_turn(&agent.driver).await;
    let summary = format!(r#""summary":"{ACCOUNT_SIGNED_OUT}""#);
    for part in [r#""kind":"auth""#, r#""accountId":"b""#, summary.as_str()] {
        assert!(notice.contains(part), "{part}: {notice}");
    }
    assert!(
        !notice.contains("The provider refused this connection"),
        "{notice}"
    );
    assert!(!notice.contains(DEFAULTS_WORDS), "{notice}");
}

/// The vault's own store says a locked keychain in its words, and a sign-in
/// read by Rig says it in Rig's.
#[tokio::test]
async fn a_locked_keychain_fails_the_chosen_account_with_its_own_words() {
    const DEFAULTS_WORDS: &str = "the default credential";
    for locked in [KEYCHAIN_LOCKED, "token store failed: keychain locked"] {
        let keys = Arc::new(keys(unavailable(DEFAULTS_WORDS), unavailable(locked)));
        let agent = started("keychain", keys.clone(), Some(account_b())).await;
        let notice = failed_turn(&agent.driver).await;
        for part in [r#""kind":"keychain""#, locked, r#""accountId":"b""#] {
            assert!(notice.contains(part), "{part}: {notice}");
        }
        assert!(!notice.contains(DEFAULTS_WORDS), "{notice}");
        assert!(
            lock(&keys.asked)
                .iter()
                .all(|asked| *asked == Some(account_b()))
        );
    }
}

#[tokio::test]
async fn a_failed_turn_names_the_default_account_only_when_its_provider_has_others() {
    for (default_account, named) in [
        (Some(("a".to_string(), true)), true),
        (Some(("a".to_string(), false)), false),
        (None, false),
    ] {
        let keys = Arc::new(Keys {
            default_account,
            ..keys(unavailable("x"), unavailable("x"))
        });
        let agent = started("default-named", keys, None).await;
        let notice = failed_turn(&agent.driver).await;
        assert_eq!(notice.contains(r#""accountId":"a""#), named, "{notice}");
        assert_eq!(notice.contains("accountId"), named, "{notice}");
    }
}

/// A chat-completions server that answers its first request and refuses
/// every later one.
async fn answers_once() -> String {
    use axum::{Router, http::StatusCode, response::IntoResponse, routing::post};
    const REPLY: &str = concat!(
        r#"data: {"id":"c","object":"chat.completion.chunk","created":1,"model":"stub","choices":[{"index":0,"delta":{"role":"assistant","content":"Free."},"finish_reason":null}]}"#,
        "\n\n",
        r#"data: {"id":"c","object":"chat.completion.chunk","created":1,"model":"stub","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    let answered = Arc::new(AtomicBool::new(false));
    let app = Router::new().route(
        "/chat/completions",
        post(move || async move {
            if answered.swap(true, Ordering::SeqCst) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            ([("content-type", "text/event-stream")], REPLY).into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn only_a_failed_turn_asks_for_the_default_account() {
    let provider = format!("custom-{}", uuid::Uuid::new_v4());
    let stub = ProviderAuth::Custom {
        name: "Stub".to_string(),
        base_url: answers_once().await,
        api_key: None,
        config: crate::contract::CustomProvider {
            api: crate::contract::OpenAiApi::ChatCompletions,
            models: vec!["stub".to_string()],
        },
    };
    let keys = Arc::new(Keys {
        default: HashMap::from([(provider.clone(), stub)]),
        default_account: Some(("a".to_string(), true)),
        ..Keys::default()
    });
    let agent = started("only-failed", keys.clone(), None).await;
    agent
        .driver
        .set_model(&format!("{provider}/stub"))
        .await
        .unwrap();

    let (stop_reason, notice) = turn(&agent.driver).await;
    assert_ne!(stop_reason, "failed", "{notice:?}");
    assert_eq!(keys.defaults_asked.load(Ordering::SeqCst), 0);
    let notice = failed_turn(&agent.driver).await;
    assert!(notice.contains(r#""accountId":"a""#), "{notice}");
    assert_eq!(keys.defaults_asked.load(Ordering::SeqCst), 1);
}
