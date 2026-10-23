use super::build::{effort_params, openrouter_model};
use super::*;
use crate::contract::Reach;
use rig::providers::anthropic;
use rig::providers::openai::{OpenAIConfig, wire};
use rig::tool::{Tool, ToolContext};
use serde_json::json;

/// A tool set of `tools`, as a turn registers them.
fn tool_set(tools: impl IntoIterator<Item = DynamicTool>) -> rig::tool::ToolSet {
    let mut set = rig::tool::ToolSet::default();
    for tool in tools {
        set.add_dynamic_tool(tool);
    }
    set
}

fn choice(id: &str) -> ConfigChoice {
    ConfigChoice {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        group: None,
    }
}

#[test]
fn saved_model_and_room_preference_survive_missing_or_filtered_choices() {
    let choices = [choice("other")];
    assert_eq!(
        model_for(Some("chosen"), Some("default"), &choices).as_deref(),
        Some("chosen")
    );
    assert_eq!(
        model_for(None, Some("default"), &choices).as_deref(),
        Some("default")
    );
    assert_eq!(
        model_for(Some("chosen"), Some("default"), &[]).as_deref(),
        Some("chosen")
    );
    assert_eq!(
        model_for(None, Some("default"), &[]).as_deref(),
        Some("default")
    );
    assert_eq!(model_for(None, None, &choices).as_deref(), Some("other"));
    assert_eq!(model_for(None, None, &[]), None);
}

#[tokio::test]
async fn native_anthropic_requests_use_live_limits_or_a_conservative_unknown_budget() {
    use axum::{Router, body::Bytes, routing::post};
    use serde_json::json;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let app = Router::new().route("/v1/messages", post(move |body: Bytes| {
        let tx = tx.clone();
        async move {
            tx.send(serde_json::from_slice::<serde_json::Value>(&body).unwrap()).await.unwrap();
            json!({"id":"msg_test", "type":"message", "role":"assistant", "model":"brand-new-claude", "content":[{"type":"text","text":"ok"}], "stop_reason":"end_turn", "stop_sequence":null, "usage":{"input_tokens":2,"output_tokens":1}}).to_string()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = anthropic::AnthropicConfig::new("test-only-key")
        .with_base_url(&url)
        .client();
    let keys = HashMap::from([("anthropic".into(), ProviderAuth::ApiKey("test-key".into()))]);
    let known = models::catalog().providers["anthropic"]
        .models
        .keys()
        .next()
        .unwrap();
    for (id, live_limit, expected) in [
        ("brand-new-claude", None, 4096),
        ("brand-new-claude", Some(1234), 1234),
        (known.as_str(), Some(1234), 1234),
    ] {
        let model_id = format!("anthropic/{id}");
        let settings = request_settings(&keys, &model_id, None, live_limit, Reuse::Once).unwrap();
        let agent = settings
            .on_agent(rig::agent::AgentBuilder::new(client.completion(id)))
            .build();
        assert_eq!(agent.prompt("hello").await.unwrap().output(), "ok");
        let request = rx.recv().await.unwrap();
        assert_eq!(request["model"], id);
        assert_eq!(request["max_tokens"], expected);
    }
    assert_eq!(models::output_limit("anthropic/brand-new-claude"), None);
    assert_eq!(
        models::efforts("anthropic/brand-new-claude"),
        [] as [String; 0]
    );
    server.abort();
}

/// A provider that answers every request on `path` with `answer` and
/// hands each request's body to the test.
async fn provider_at(
    path: &'static str,
    answer: serde_json::Value,
) -> (
    String,
    tokio::sync::mpsc::Receiver<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    use axum::{Router, body::Bytes, routing::post};
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let answer = answer.to_string();
    let app = Router::new().route(
        path,
        post(move |body: Bytes| {
            let tx = tx.clone();
            let answer = answer.clone();
            async move {
                tx.send(serde_json::from_slice(&body).unwrap())
                    .await
                    .unwrap();
                answer
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, rx, server)
}

#[tokio::test]
async fn claude_on_openrouter_is_asked_to_cache_and_other_models_are_left_alone() {
    let (url, mut requests, server) = provider_at(
        "/chat/completions",
        serde_json::json!({
            "id":"gen-test","object":"chat.completion","created":1,"model":"routed",
            "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        }),
    )
    .await;
    let config = OpenAIConfig::with_key(&wire::OPENROUTER, "test-only-key").with_base_url(&url);
    for (model, reuse, cached) in [
        ("anthropic/claude-test", Reuse::Rounds, true),
        ("anthropic/claude-test", Reuse::Once, false),
        ("openai/gpt-test", Reuse::Rounds, false),
    ] {
        let agent =
            rig::agent::AgentBuilder::new(openrouter_model(config.clone(), model, reuse).unwrap())
                .preamble("You are Ada.")
                .build();
        assert_eq!(agent.prompt("hello").await.unwrap().output(), "ok");
        let request = requests.recv().await.unwrap();
        let preamble = &request["messages"][0];
        assert_eq!(preamble["role"], "system", "{request}");
        assert_eq!(
            preamble["content"][0].get("cache_control").is_some(),
            cached,
            "{model} {reuse:?}: {request}"
        );
    }
    server.abort();
}

#[test]
fn attachments_reach_this_agent_as_paths_under_the_message() {
    use crate::contract::AttachmentKind;
    let attachment = Attachment {
        created: None,
        kind: AttachmentKind::File,
        name: "note.txt".to_string(),
        path: "/tmp/note.txt".to_string(),
        mime_type: None,
        size: None,
        width: None,
        height: None,
        origin: None,
    };
    assert_eq!(
        user_message("look", std::slice::from_ref(&attachment)),
        Message::user("look\n\nAttached files:\n/tmp/note.txt")
    );
}

/// A picture reaches the model as pixels beside the path, whoever the
/// provider is; one too big to send, or missing, is named as such and
/// stays a path.
#[test]
fn an_attached_image_reaches_the_model_as_pixels_and_as_a_path() {
    use crate::contract::AttachmentKind;
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("shot.png");
    image::RgbImage::new(8, 8).save(&png).unwrap();
    let image = |path: &Path| Attachment {
        created: None,
        kind: AttachmentKind::Image,
        name: "shot.png".to_string(),
        path: path.to_string_lossy().into_owned(),
        mime_type: Some("image/png".to_string()),
        size: None,
        width: None,
        height: None,
        origin: None,
    };
    let Message::User { content } = user_message("see", &[image(&png)]) else {
        panic!("a user message");
    };
    assert_eq!(content.len(), 2);
    assert_eq!(
        content[0],
        UserContent::text(format!("see\n\nAttached files:\n{}", png.display()))
    );
    let UserContent::Image(sent) = &content[1] else {
        panic!("an image block");
    };
    assert_eq!(sent.media_type, Some(ImageMediaType::JPEG));

    // A photo-sized picture arrives as a JPEG no wider than the edge cap.
    let wide = dir.path().join("wide.png");
    image::RgbImage::from_fn(3000, 300, |x, _| image::Rgb([(x % 256) as u8, 40, 200]))
        .save(&wide)
        .unwrap();
    let Message::User { content } = user_message("see", &[image(&wide)]) else {
        panic!("a user message");
    };
    let UserContent::Image(sent) = &content[1] else {
        panic!("an image block");
    };
    assert_eq!(sent.media_type, Some(ImageMediaType::JPEG));
    let DocumentSourceKind::Base64(data) = &sent.data else {
        panic!("inline bytes");
    };
    use base64::{Engine, prelude::BASE64_STANDARD};
    let shrunk = image::load_from_memory(&BASE64_STANDARD.decode(data).unwrap()).unwrap();
    assert_eq!((shrunk.width(), shrunk.height()), (2000, 200));

    let missing = image(&dir.path().join("gone.png"));
    let Message::User { content } = user_message("see", std::slice::from_ref(&missing)) else {
        panic!("a user message");
    };
    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0],
        UserContent::text(format!(
            "see\n\nAttached files:\n{} (not readable; on disk only)",
            missing.path
        ))
    );
}

#[test]
fn a_tool_call_is_described_by_what_it_touched() {
    assert_eq!(
        describe_tool("read", &json!({"path": "src/x.rs"})),
        "read src/x.rs"
    );
    assert_eq!(
        describe_tool("shell", &json!({"command": "ls"})),
        "shell ls"
    );
    assert_eq!(describe_tool("glob", &json!({})), "glob");
}

#[test]
fn only_providers_that_take_a_picture_in_a_tool_result_get_one() {
    assert!(images_to_model("anthropic"));
    assert!(images_to_model("openai-codex"));
    assert!(!images_to_model("openrouter"));
    assert!(!images_to_model("ollama"));
}

const BUDGET_100: Budget = Budget { head: 50, tail: 50 };

#[test]
fn output_under_the_limit_is_untouched() {
    assert_eq!(elide("hello", Budget { head: 8, tail: 8 }), "hello");
}

/// The head and the tail both survive, and the line between them says how
/// much did not.
#[test]
fn a_long_output_keeps_both_ends_and_says_what_it_cut() {
    let text = format!("start{}end", "x".repeat(1_000));
    let elided = elide(&text, BUDGET_100);
    assert!(elided.starts_with("start"));
    assert!(elided.ends_with("end"));
    assert!(elided.contains("[… 908 bytes elided …]"), "{elided}");
}

/// A log is cut between lines when a line ends near the cut.
#[test]
fn a_log_is_cut_between_lines() {
    let text: String = (0..100).map(|i| format!("line {i:03}\n")).collect();
    let elided = elide(&text, BUDGET_100);
    assert!(
        elided.starts_with("line 000\nline 001\nline 002\nline 003\nline 004\n[… "),
        "{elided}"
    );
    assert!(
        elided.ends_with(" elided …]\nline 095\nline 096\nline 097\nline 098\nline 099\n"),
        "{elided}"
    );
}

/// A command's stdout and stderr each keep their start and end, and a
/// short one is kept whole while the other takes the rest.
#[test]
fn a_commands_streams_are_cut_each_to_its_own_ends() {
    use crate::tools::STDERR_LINE;
    let stdout: String = (0..5000)
        .map(|i| format!("test case_{i:04} ... ok\n"))
        .collect();
    let stderr: String = (0..5000)
        .map(|i| format!("   Compiling crate-{i:04}\n"))
        .collect();
    let view = command_view(&format!(
        "{stdout}summary{STDERR_LINE}{stderr}error\n[exit status 101]"
    ));
    assert!(
        view.len() < Budget::COMMAND.bytes() + 200,
        "{} bytes",
        view.len()
    );
    assert!(view.starts_with("test case_0000 ... ok\n"));
    assert!(
        view.contains(&format!(
            "case_4999 ... ok\nsummary{STDERR_LINE}   Compiling crate-0000\n"
        )),
        "{view}"
    );
    assert!(
        view.ends_with("crate-4999\nerror\n[exit status 101]"),
        "{view}"
    );
    let view = command_view(&format!("short{STDERR_LINE}{stderr}"));
    assert!(
        view.starts_with(&format!("short{STDERR_LINE}   Compiling crate-0000\n")),
        "{view}"
    );
    assert!(
        view.len() > Budget::COMMAND.bytes() - 100,
        "{} bytes",
        view.len()
    );
    let view = command_view(&stderr);
    assert_eq!(view.matches("elided").count(), 1, "{view}");
}

/// Colour, redraws and titles are gone; the lines they drew are not.
#[test]
fn a_commands_output_reads_as_a_terminal_left_it() {
    let raw = "\u{1b}[1m\u{1b}[92m   Compiling\u{1b}[0m ctor v0.8.0\n\u{1b}[96mBuilding\u{1b}[0m 1/9\rBuilding 2/9\u{1b}[K\n";
    assert_eq!(plain(raw), "   Compiling ctor v0.8.0\nBuilding 2/9\n");
    let raw = "\u{1b}]0;make\u{7}one\r\n\u{1b}(B\u{1b}[mtwo\u{1b}[2~\r\n";
    assert_eq!(plain(raw), "one\ntwo\n");
    assert_eq!(plain("a\u{1b}\nb\u{1b}"), "a\nb");
}

#[test]
fn a_granted_servers_tools_get_the_external_budget() {
    assert_eq!(Budget::of("computer__shell"), Budget::EXTERNAL);
    assert_eq!(Budget::of("linear__get_issue"), Budget::EXTERNAL);
    assert_eq!(Budget::of(granted::CALL), Budget::EXTERNAL);
    assert_eq!(Budget::of("read"), Budget::BUILT_IN);
    assert_eq!(Budget::of("search_thread"), Budget::BUILT_IN);
}

/// The cut lands on a character boundary, never inside one.
#[test]
fn a_multibyte_output_is_cut_between_characters() {
    let text = "é".repeat(1_000);
    let elided = elide(&text, Budget { head: 51, tail: 50 });
    assert!(elided.contains("[…"));
    assert!(elided.starts_with('é'));
    assert!(elided.ends_with('é'));
}

/// A command whose output exceeds what the model is handed lands its full
/// output on disk, and the text the model sees names that path.
#[tokio::test]
async fn a_shell_command_whose_output_exceeds_the_elision_lands_on_disk() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "hotline-core-tool-output-{}-{nonce}",
        std::process::id()
    ));
    let workspace_dir = root.join("workspace");
    let output_dir = root.join("tool-output");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let body = "x".repeat(Budget::BUILT_IN.bytes() + 64);
    std::fs::write(workspace_dir.join("big.txt"), &body).unwrap();
    let workspace = Workspace::open(workspace_dir, Reach::Machine, output_dir.clone()).unwrap();
    let args: <RunCommand as Tool>::Args =
        serde_json::from_value(json!({"command": "cat big.txt"})).unwrap();
    let output = RunCommand::new(workspace)
        .call(&mut ToolContext::new(), args)
        .await
        .unwrap();
    assert_eq!(output, body);

    let handed = hand_to_model(&output_dir, "call-1", &output, Budget::BUILT_IN);
    let saved = output_dir.join("call-1.txt");
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), body);
    let named = saved.canonicalize().unwrap();
    assert!(handed.contains(&named.display().to_string()), "{handed}");
    assert!(handed.contains("elided"), "{handed}");
    assert!(handed.starts_with('x') && handed.contains("Full output:"));
    let _ = std::fs::remove_dir_all(&root);
}

/// The bug this replaced: a turn is only registered on the stop while it
/// is polling for the next stream item, and it spends the first model
/// round trip and every full-channel push registered nowhere. A wake sent
/// in one of those moments reached nobody and the turn ran on.
#[tokio::test]
async fn a_stop_pressed_before_the_turn_waits_on_it_is_still_heard() {
    let stop = Stop::default();
    stop.raise();
    tokio::time::timeout(std::time::Duration::from_secs(5), stop.raised())
        .await
        .expect("the turn was never told to stop");
}

/// And one pressed while the turn is waiting wakes it there.
#[tokio::test]
async fn a_stop_pressed_while_the_turn_waits_wakes_it() {
    let stop = Arc::new(Stop::default());
    let waiting = tokio::spawn({
        let stop = stop.clone();
        async move { stop.raised().await }
    });
    tokio::task::yield_now().await;
    stop.raise();
    tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("the turn was never told to stop")
        .unwrap();
}

/// A scripted failure (a provider this agent does not speak) used to
/// drop the user's line, so the next turn reached the model with no
/// record of the question. Cancel already kept it.
#[tokio::test]
async fn a_failed_turn_keeps_the_users_line_so_a_retry_still_has_the_question() {
    let root = std::env::temp_dir().join(format!(
        "hotline-core-rig-fail-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let history = Arc::new(AsyncMutex::new(Vec::new()));
    let turn = Turn {
        output_limit: None,
        context_limit: None,
        meter: None,
        keys: HashMap::new(),
        model: "nope/none".to_string(),
        effort: None,
        preamble: "you are Ada".to_string(),
        cwd: root.clone(),
        folders: Vec::new(),
        reach: Reach::Workspace,
        history: history.clone(),
        stop: Arc::new(Stop::default()),
        steering: Arc::new(turn::Steering::default()),
        output_dir: root.join("out"),
        mcp_tools: Vec::new(),
        capability: None,
        delegate: None,
        processes: crate::session::jobs::Processes::new(std::env::temp_dir()),
    };
    let (sender, _receiver) = mpsc::channel(8);
    let result = turn
        .run(&sender, Some(Message::user("did the crane jam?")))
        .await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        *history.lock().await,
        vec![Message::user("did the crane jam?")]
    );
    // A retry runs on that history as it is: the question is not asked twice.
    let again = turn.run(&sender, None).await;
    assert!(again.is_err(), "{again:?}");
    assert_eq!(
        *history.lock().await,
        vec![Message::user("did the crane jam?")]
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn login_scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hotline-core-rig-login-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A turn never starts a login: empty ChatGPT auth with the device flow
/// disallowed fails locally, before any network call.
#[tokio::test]
async fn a_chatgpt_turn_without_a_login_refuses_instead_of_starting_one() {
    let dir = login_scratch("chatgpt");
    std::fs::write(dir.join("auth.json"), "{}").unwrap();
    let keys = HashMap::from([(
        "openai-codex".to_string(),
        ProviderAuth::Login {
            token_dir: dir.clone(),
        },
    )]);
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        complete(&keys, "openai-codex/gpt-5.6", "you are Ada", "hello", None),
    )
    .await
    .expect("ChatGPT auth without a token must not wait on the network")
    .expect_err("empty auth.json must not complete");
    assert!(
        err.to_ascii_lowercase().contains("sign-in"),
        "a turn must name the missing sign-in, not start one: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Same proof for Copilot: a `{}` record, with the device flow disallowed,
/// fails locally.
#[tokio::test]
async fn a_copilot_turn_without_a_login_refuses_instead_of_starting_one() {
    let dir = login_scratch("copilot");
    std::fs::write(dir.join(crate::providers::rig_login::COPILOT_RECORD), "{}").unwrap();
    let keys = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: dir.clone(),
        },
    )]);
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        complete(
            &keys,
            "github-copilot/gpt-5-mini",
            "you are Ada",
            "hello",
            None,
        ),
    )
    .await
    .expect("Copilot auth without a token must not wait on the network")
    .expect_err("empty Copilot files must not complete");
    assert!(
        err.to_ascii_lowercase().contains("sign-in"),
        "a turn must name the missing sign-in, not start one: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Copilot model the account offers only on `/responses` is driven
/// there, signed as Copilot expects, with the effort in the Responses
/// shape; one it never named stays on Rig's `/chat/completions` route,
/// on the same stored token.
#[tokio::test]
async fn a_responses_only_copilot_model_takes_the_responses_route() {
    use axum::{
        Router,
        body::Bytes,
        http::HeaderMap,
        routing::{get, post},
    };
    use serde_json::json;
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let responses = tx.clone();
    let listed = tx.clone();
    let app = Router::new()
        .route(
            "/models",
            get(move |headers: HeaderMap| {
                let tx = listed.clone();
                async move {
                    tx.send(("/models", headers, serde_json::Value::Null)).await.unwrap();
                    json!({"data": [
                        {"id":"grok-4.6","supported_endpoints":["/responses"]},
                        {"id":"claude-sonnet-5","supported_endpoints":["/chat/completions","/responses"]}
                    ]})
                    .to_string()
                }
            }),
        )
        .route(
            "/responses",
            post(move |headers: HeaderMap, body: Bytes| {
                let tx = responses.clone();
                async move {
                    tx.send(("/responses", headers, serde_json::from_slice::<serde_json::Value>(&body).unwrap())).await.unwrap();
                    json!({
                        "id":"resp_1","object":"response","created_at":1,"status":"completed","model":"grok-4.6",
                        "output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed",
                            "content":[{"type":"output_text","text":"over responses"}]}],
                        "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}
                    }).to_string()
                }
            }),
        )
        .route(
            "/chat/completions",
            post(move |headers: HeaderMap, body: Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send(("/chat/completions", headers, serde_json::from_slice::<serde_json::Value>(&body).unwrap())).await.unwrap();
                    json!({
                        "id":"chatcmpl_1","object":"chat.completion","created":1,"model":"claude-sonnet-5",
                        "choices":[{"index":0,"message":{"role":"assistant","content":"over chat"},"finish_reason":"stop"}],
                        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
                    }).to_string()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let dir = crate::providers::copilot::tests::seeded_login("route", &url);
    crate::providers::copilot::tests::record_endpoints(
        &dir,
        json!({"grok-4.6": ["/responses"], "claude-sonnet-5": ["/chat/completions"]}),
    );
    let keys = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: dir.clone(),
        },
    )]);

    let answer = complete(
        &keys,
        "github-copilot/grok-4.6",
        "you are Ada",
        "hello",
        None,
    )
    .await
    .unwrap();
    assert_eq!(answer, "over responses");
    let (path, headers, body) = rx.recv().await.unwrap();
    assert_eq!(path, "/responses");
    assert_eq!(body["model"], "grok-4.6");
    assert_eq!(headers["authorization"], "Bearer copilot-test-token");
    assert_eq!(headers["copilot-integration-id"], "vscode-chat");
    assert_eq!(headers["x-initiator"], "user");
    assert!(
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["role"] == "system"),
        "the preamble rides as an input message, as on Rig's Copilot route: {body}"
    );

    let answer = complete(
        &keys,
        "github-copilot/claude-sonnet-5",
        "you are Ada",
        "hello",
        None,
    )
    .await
    .unwrap();
    assert_eq!(answer, "over chat");
    let (path, _, body) = rx.recv().await.unwrap();
    assert_eq!(path, "/chat/completions");
    assert_eq!(body["model"], "claude-sonnet-5");

    let effort = |model: &str| {
        request_settings(&keys, model, Some("high"), None, Reuse::Rounds)
            .unwrap()
            .additional_params
    };
    assert_eq!(
        effort("github-copilot/grok-4.6"),
        Some(json!({"reasoning": {"effort": "high"}}))
    );
    assert_eq!(
        effort("github-copilot/claude-sonnet-5"),
        Some(json!({"reasoning_effort": "high"}))
    );
    assert_eq!(
        effort("github-copilot/gpt-5.3-codex"),
        Some(json!({"reasoning": {"effort": "high"}}))
    );

    // Tools on the Responses route are strict, as on Rig's own route.
    let model = completion_model(&keys, "github-copilot/grok-4.6", Reuse::Rounds)
        .await
        .unwrap();
    let request = rig::completion::CompletionRequest::new("read it").tool(
        rig::completion::ToolDefinition::new(
            rig::message::ToolName::new("read_file").unwrap(),
            "Read a file",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        ),
    );
    model.call(request).await.unwrap();
    let (path, _, body) = rx.recv().await.unwrap();
    assert_eq!(path, "/responses");
    assert_eq!(body["tools"][0]["name"], "read_file");
    assert_eq!(body["tools"][0]["strict"], true, "{body}");
    assert_eq!(
        body["tools"][0]["parameters"]["additionalProperties"],
        false
    );

    // A login from before endpoints were recorded fetches them on its
    // first turn and takes the Responses route without a Refresh.
    let older = crate::providers::copilot::tests::seeded_login("route-older", &url);
    let older_keys = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: older.clone(),
        },
    )]);
    let answer = complete(
        &older_keys,
        "github-copilot/grok-4.6",
        "you are Ada",
        "hello",
        None,
    )
    .await
    .unwrap();
    assert_eq!(answer, "over responses");
    assert_eq!(rx.recv().await.unwrap().0, "/models");
    assert_eq!(rx.recv().await.unwrap().0, "/responses");
    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&older);
}

fn echo_command() -> String {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_hotline_mcp_echo") {
        return path;
    }
    // A unit test is not told where its bins are, only an integration
    // test is; but it knows where it is itself, and the bins are one
    // directory up from `deps`, wherever the target directory lives.
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let name = format!("hotline-mcp-echo{}", std::env::consts::EXE_SUFFIX);
            Some(exe.parent()?.parent()?.join(name))
        })
        .filter(|path| path.exists())
        .expect("hotline-mcp-echo should have been built with this test")
        .to_string_lossy()
        .into_owned()
}

/// The failing call returns the error to the model as before, and says
/// nothing to the conversation: the ledger already carries it.
#[tokio::test]
async fn a_dead_mcp_transport_fails_the_call_and_nothing_else() {
    use crate::mcp::{McpServer, McpTransport};

    let connected = mcp::connect(
        "mcp-notice-turn",
        &[McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: echo_command(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }],
    )
    .await;
    assert!(connected.failed.is_empty(), "{:?}", connected.failed);
    let tool = connected.tools[0].clone();
    let name = tool.name.clone();
    let set = tool_set([mcp_dynamic(tool)]);
    let first = set
        .execute(
            &name,
            json!({"text": "harbour"}).to_string(),
            &mut ToolContext::new(),
        )
        .await;
    assert!(first.is_success(), "{first:?}");

    drop(connected);
    let second = set
        .execute(
            &name,
            json!({"text": "harbour"}).to_string(),
            &mut ToolContext::new(),
        )
        .await;
    assert!(!second.is_success(), "{second:?}");
}

/// A granted server's tools reach the model as two fixed tools and a
/// line each in the preamble; the schema is read on demand and the call
/// lands on the server as if the tool had been in the list.
#[tokio::test]
async fn a_granted_tool_is_read_and_called_through_the_fixed_pair() {
    use crate::mcp::{McpServer, McpTransport};

    let connected = mcp::connect(
        "mcp-granted-pair",
        &[McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: echo_command(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }],
    )
    .await;
    assert!(connected.failed.is_empty(), "{:?}", connected.failed);
    let echo = connected.tools.clone();
    assert!(echo.iter().all(granted::is_granted));
    let name = echo[0].name.clone();

    // The list the model sees does not depend on what is granted.
    let definitions = |tools: Vec<mcp::McpTool>| tool_set(granted::tools(tools)).tool_definitions();
    assert_eq!(definitions(echo.clone()), definitions(Vec::new()));
    let index = granted::index(&echo);
    assert!(
        index.contains(&format!("- {name}: Echo the text back in upper case.")),
        "{index}"
    );
    assert_eq!(granted::index(&[]), "");

    let set = tool_set(granted::tools(echo));
    let schema = set
        .execute(
            granted::SCHEMA,
            json!({"tool": name}).to_string(),
            &mut ToolContext::new(),
        )
        .await;
    assert!(schema.is_success(), "{schema:?}");
    let schema = schema.output().render();
    assert!(schema.contains("Parameters (JSON Schema)"), "{schema}");
    assert!(schema.contains("\"text\""), "{schema}");

    let called = set
        .execute(
            granted::CALL,
            json!({"tool": name, "arguments": "{\"text\": \"harbour\"}"}).to_string(),
            &mut ToolContext::new(),
        )
        .await;
    assert!(called.is_success(), "{called:?}");
    assert!(called.output().render().contains("HARBOUR"), "{called:?}");

    let unknown = set
        .execute(
            granted::CALL,
            json!({"tool": "echo__whisper", "arguments": "{}"}).to_string(),
            &mut ToolContext::new(),
        )
        .await;
    assert!(!unknown.is_success(), "{unknown:?}");
    let garbled = set
        .execute(
            granted::CALL,
            json!({"tool": name, "arguments": "text=harbour"}).to_string(),
            &mut ToolContext::new(),
        )
        .await;
    assert!(!garbled.is_success(), "{garbled:?}");
}

/// A CallToolResult with text and an image becomes text plus an image
/// in the Rig output, so the model sees the picture.
#[test]
fn an_mcp_image_block_reaches_the_rig_output() {
    use crate::mcp::{CallContent, CallImage};
    use rig::message::{DocumentSourceKind, ImageMediaType, ToolResultContent};

    let output = rig_output(CallContent {
        text: "the tree".into(),
        images: vec![CallImage {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
        }],
    })
    .expect("mixed content is a valid tool output");
    let content = output.as_content();
    assert!(
        matches!(
            content,
            [
                ToolResultContent::Text(text),
                ToolResultContent::Image(image)
            ] if text.text == "the tree"
                && image.media_type == Some(ImageMediaType::PNG)
                && matches!(&image.data, DocumentSourceKind::Base64(data) if data == "AAAA")
        ),
        "{content:?}"
    );
}

#[test]
fn effort_params_per_client() {
    assert_eq!(
        effort_params(Client::OllamaCloud, "max"),
        Some(serde_json::json!({"think": "max"}))
    );
    assert_eq!(
        effort_params(Client::Ollama, "medium"),
        Some(serde_json::json!({"think": "medium"}))
    );
    assert_eq!(effort_params(Client::OllamaCloud, "xhigh"), None);

    assert_eq!(
        effort_params(Client::Anthropic, "high"),
        Some(json!({
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "high"}
        }))
    );
    let reasoning = json!({"reasoning": {"effort": "high"}});
    assert_eq!(
        effort_params(Client::OpenAi, "high"),
        Some(reasoning.clone())
    );
    assert_eq!(
        effort_params(Client::ChatGpt, "high"),
        Some(reasoning.clone())
    );
    assert_eq!(
        effort_params(Client::Copilot, "high"),
        Some(reasoning.clone())
    );
    assert_eq!(
        effort_params(Client::OpenRouter, "high"),
        Some(reasoning.clone())
    );
    assert_eq!(effort_params(Client::XAi, "high"), Some(reasoning));
    assert_eq!(
        effort_params(Client::Gemini, "high"),
        Some(json!({
            "generationConfig": {"thinkingConfig": {"thinkingLevel": "high"}}
        }))
    );
    assert_eq!(effort_params(Client::Gemini, "xhigh"), None);
    let chat = json!({"reasoning_effort": "high"});
    assert_eq!(effort_params(Client::Groq, "high"), Some(chat.clone()));
    assert_eq!(effort_params(Client::DeepSeek, "high"), Some(chat.clone()));
    assert_eq!(effort_params(Client::Mistral, "high"), Some(chat));
}

/// The standup that posted on every turn: on Copilot, a teammate's own
/// earlier reply must reach the next request as text the model can read.
/// Copilot drops an assistant `content` list on the way to Claude, so the
/// reply goes as a string.
#[tokio::test]
async fn a_copilot_teammate_hears_its_own_last_reply_on_the_next_turn() {
    use axum::{Router, body::Bytes, routing::post};
    const CALL: &str = concat!(
        r#"data: {"choices":[{"index":0,"delta":{"content":"On it.","role":"assistant"}}]}"#,
        "\n\n",
        r#"data: {"choices":[{"index":0,"delta":{"content":null,"tool_calls":[{"function":{"name":"ls"},"id":"tooluse_1","index":1,"type":"function"}]}}]}"#,
        "\n\n",
        r#"data: {"choices":[{"index":0,"delta":{"content":null,"tool_calls":[{"function":{"arguments":""},"index":1,"type":"function"}]}}]}"#,
        "\n\n",
        r#"data: {"choices":[{"finish_reason":"tool_calls","index":0,"delta":{"content":null}}],"usage":{"completion_tokens":1,"prompt_tokens":1,"total_tokens":2}}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    const SAY: &str = concat!(
        r#"data: {"choices":[{"index":0,"delta":{"content":"Today — Mon 5 Oct","role":"assistant"}}]}"#,
        "\n\n",
        r#"data: {"choices":[{"finish_reason":"stop","index":0,"delta":{"content":null}}],"usage":{"completion_tokens":1,"prompt_tokens":1,"total_tokens":2}}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let app = Router::new().route(
        "/chat/completions",
        post(move |body: Bytes| {
            let tx = tx.clone();
            async move {
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let opening = body["messages"].as_array().unwrap().len() == 2;
                tx.send(body).unwrap();
                (
                    [("content-type", "text/event-stream")],
                    if opening { CALL } else { SAY },
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = crate::providers::copilot::tests::seeded_login("own-reply", &url);
    crate::providers::copilot::tests::record_endpoints(
        &dir,
        json!({"claude-opus-5.5": ["/chat/completions"]}),
    );
    let root = login_scratch("own-reply");
    let history = Arc::new(AsyncMutex::new(Vec::new()));
    let turn = || Turn {
        output_limit: None,
        context_limit: None,
        meter: None,
        keys: HashMap::from([(
            "github-copilot".to_string(),
            ProviderAuth::Login {
                token_dir: dir.clone(),
            },
        )]),
        model: "github-copilot/claude-opus-5.5".to_string(),
        effort: None,
        preamble: "you are Ada".to_string(),
        cwd: root.clone(),
        folders: Vec::new(),
        reach: Reach::Workspace,
        history: history.clone(),
        stop: Arc::new(Stop::default()),
        steering: Arc::new(turn::Steering::default()),
        output_dir: root.join("out"),
        mcp_tools: Vec::new(),
        capability: None,
        delegate: None,
        processes: crate::session::jobs::Processes::new(std::env::temp_dir()),
    };
    let (sender, mut receiver) = mpsc::channel(256);
    tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    turn()
        .run(&sender, Some(Message::user("run the standup")))
        .await
        .unwrap();
    turn()
        .run(&sender, Some(Message::user("have a peek")))
        .await
        .unwrap();
    let mut bodies = Vec::new();
    while let Ok(body) = rx.try_recv() {
        bodies.push(body);
    }
    assert_eq!(bodies.len(), 3);
    let said: Vec<&serde_json::Value> = bodies[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "assistant")
        .map(|message| &message["content"])
        .collect();
    assert_eq!(said, [&json!("On it."), &json!("Today — Mon 5 Oct")]);
    let _ = std::fs::remove_dir_all(&root);
}

/// The eight desktop tools, as the computer image lists them.
const COMPUTER_TOOLS: [&str; 8] = [
    "capture", "input", "browser", "shell", "files", "windows", "wait", "state",
];

struct Desktop;

impl rmcp::handler::server::ServerHandler for Desktop {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        let schema = Arc::new(
            json!({"type": "object"})
                .as_object()
                .expect("an object")
                .clone(),
        );
        std::future::ready(Ok(rmcp::model::ListToolsResult::with_all_items(
            COMPUTER_TOOLS
                .iter()
                .map(|name| rmcp::model::Tool::new(*name, *name, schema.clone()))
                .collect(),
        )))
    }
}

fn test_persona(id: &str) -> crate::contract::Persona {
    crate::contract::Persona {
        node: None,
        id: id.to_string(),
        name: "Ada".to_string(),
        goal: String::new(),
        avatar: None,
        team: None,
        backend_id: "hotline".to_string(),
        cwd: "/".to_string(),
        reach: Some(crate::contract::Reach::Machine),
        model_id: None,
        mode_id: None,
        effort_id: None,
        harness_override: None,
        account_id: None,
        hop_notice: None,
        mcp_policy: crate::contract::McpPolicy {
            mode: crate::contract::PolicyMode::All,
            server_ids: Vec::new(),
        },
        skill_policy: crate::contract::SkillPolicy::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: None,
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1,
        updated_at: 1,
    }
}

/// The computer is a granted server like any other: its eight desktop tools,
/// served over HTTP as the image serves them, land on the ledger under the
/// origin `computer`, verified.
#[tokio::test(flavor = "multi_thread")]
async fn a_computer_server_writes_its_eight_tools_on_the_ledger_under_origin_computer() {
    use crate::contract::{ToolSourceKind, ToolState};
    use crate::mcp::{HttpAuth, McpServer, McpTransport};
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let persona_id = "mcp-computer-ledger";
    let service: StreamableHttpService<Desktop, LocalSessionManager> = StreamableHttpService::new(
        || Ok(Desktop),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("loopback listener");
    let url = format!("http://{}/mcp", listener.local_addr().expect("address"));
    let served = tokio::spawn(async move {
        let _ = axum::serve(listener, axum::Router::new().nest_service("/mcp", service)).await;
    });
    let connected = mcp::connect(
        persona_id,
        &[McpServer {
            id: crate::computer::SERVER_ID.to_string(),
            name: "Computer".to_string(),
            transport: McpTransport::Http {
                url,
                auth: HttpAuth::None,
            },
            refuse: None,
        }],
    )
    .await;
    assert!(connected.failed.is_empty(), "{:?}", connected.failed);
    assert_eq!(connected.tools.len(), 8, "{}", connected.tools.len());

    publish_ledger(
        &test_persona(persona_id),
        &[],
        &connected,
        false,
        &crate::mcp::server::TOOL_NAMES,
    );
    let rows = crate::session::ledger::teammate_tools(persona_id)
        .expect("published")
        .rows;
    let computer: Vec<_> = rows
        .iter()
        .filter(|row| row.origin == crate::computer::SERVER_ID)
        .collect();
    assert_eq!(computer.len(), 8, "{rows:?}");
    for name in COMPUTER_TOOLS {
        let row = computer
            .iter()
            .find(|row| row.name == format!("computer__{name}"))
            .unwrap_or_else(|| panic!("computer__{name} is on the ledger: {rows:?}"));
        assert_eq!(row.source, ToolSourceKind::Mcp);
        assert_eq!(row.state, ToolState::Verified);
        assert_ne!(row.reason, "");
    }

    drop(connected);
    served.abort();
}

mod accounts;
mod pins;
