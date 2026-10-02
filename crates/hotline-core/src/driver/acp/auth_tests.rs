use super::tests::{agent_pipes, client_transport, persona, room, scratch, scratch_cwd};
use super::*;
use acp::{
    AuthenticateRequest, AuthenticateResponse, InitializeResponse, NewSessionResponse,
    PromptResponse, StopReason,
};
use agent_client_protocol::schema::v1::SessionNotification;
use std::sync::atomic::AtomicUsize;
use tokio_util::sync::CancellationToken;

fn fixture(
    signed: Arc<AtomicBool>,
    prompts: Arc<AtomicUsize>,
    opens: Arc<AtomicUsize>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let auth = signed.clone();
        let _ = Agent.builder()
            .on_receive_request(async |request: InitializeRequest, responder: Responder<InitializeResponse>, _cx| {
                assert_eq!(request.client_capabilities.auth.terminal, cfg!(unix));
                let methods = vec![
                    acp::AuthMethod::Agent(acp::AuthMethodAgent::new("login", "Harness login")),
                    acp::AuthMethod::Agent(acp::AuthMethodAgent::new("fail", "Fail fixture")),
                    acp::AuthMethod::Agent(acp::AuthMethodAgent::new("wait", "Wait fixture")),
                    serde_json::from_value(serde_json::json!({
                        "id": "legacy-terminal", "name": "Unsupported legacy login",
                        "_meta": {"terminal-auth": {"command": "never-execute", "args": ["private-legacy-argument"]}}
                    })).unwrap(),
                    acp::AuthMethod::Terminal(acp::AuthMethodTerminal::new("terminal", "Terminal fixture")
                        .args(vec!["private-executable-argument".into()])
                        .env(HashMap::from([("PRIVATE".into(), "private-environment-value".into())]))),
                ];
                responder.respond(InitializeResponse::new(request.protocol_version).auth_methods(methods))
            }, agent_client_protocol::on_receive_request!())
            .on_receive_request(move |_request: NewSessionRequest, responder: Responder<NewSessionResponse>, _cx| {
                let signed = signed.clone(); let opens = opens.clone();
                async move {
                    opens.fetch_add(1, Ordering::SeqCst);
                    if signed.load(Ordering::SeqCst) { responder.respond(NewSessionResponse::new("signed-in")) }
                    else { responder.respond_with_internal_error("authentication required") }
                }
            }, agent_client_protocol::on_receive_request!())
            .on_receive_request(move |request: AuthenticateRequest, responder: Responder<AuthenticateResponse>, cx: ConnectionTo<Client>| {
                let signed = auth.clone();
                async move {
                    match request.method_id.to_string().as_str() {
                        "login" => {
                            // This must not leak into the next prompt or the tape.
                            cx.send_notification(SessionNotification::new("signed-in", SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(ContentBlock::Text(acp::TextContent::new("private-auth-notification"))))))?;
                            signed.store(true, Ordering::SeqCst);
                            responder.respond(AuthenticateResponse::new())
                        },
                        "fail" => responder.respond_with_internal_error("private-auth-error"),
                        "wait" => { std::future::pending::<()>().await; responder.respond(AuthenticateResponse::new()) },
                        _ => panic!("unadvertised method reached authenticate"),
                    }
                }
            }, agent_client_protocol::on_receive_request!())
            .on_receive_request(move |_request: PromptRequest, responder: Responder<PromptResponse>, _cx| {
                let prompts = prompts.clone();
                async move { prompts.fetch_add(1, Ordering::SeqCst); responder.respond(PromptResponse::new(StopReason::EndTurn)) }
            }, agent_client_protocol::on_receive_request!())
            .connect_to(transport).await;
    }
}

async fn finished(driver: &ChildAgent, id: &str) -> crate::driver::auth::AuthStatus {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let status = driver.auth_poll(id).unwrap();
            if status.state != "running" {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sign-in settled")
}

#[tokio::test]
async fn auth_signed_out_session_is_recoverable_and_success_never_resends() {
    let held = room("auth-success");
    let driver = Arc::new(ChildAgent::new(
        scratch("auth-success"),
        "cursor".into(),
        "fixture".into(),
        TeammateTools::new(&held, "ada"),
    ));
    let prompts = Arc::new(AtomicUsize::new(0));
    let opens = Arc::new(AtomicUsize::new(0));
    let agent = tokio::spawn(fixture(
        Arc::new(AtomicBool::new(false)),
        prompts.clone(),
        opens.clone(),
    ));
    driver
        .handshake(&persona(&scratch_cwd(), vec![]), client_transport())
        .await
        .unwrap();
    let notice = driver
        .startup_failure()
        .expect("recoverable sign-in failure");
    assert!(notice.contains("signIn"));
    assert!(notice.contains("Harness login"));
    assert!(!notice.contains("private-executable"));
    assert!(!notice.contains("private-environment"));
    assert!(!notice.contains("Unsupported legacy login"));
    assert!(!notice.contains("private-legacy-argument"));
    assert!(
        driver
            .clone()
            .auth_start("legacy-terminal", CancellationToken::new())
            .await
            .is_err()
    );
    assert!(lock(&driver.live.auth).is_none());
    assert!(
        driver
            .clone()
            .auth_start("arbitrary-ui-command", CancellationToken::new())
            .await
            .is_err()
    );
    let id = driver
        .clone()
        .auth_start("login", CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(finished(&driver, &id).await.state, "succeeded");
    assert!(driver.startup_failure().is_none());
    assert!(lock(&driver.live.open).is_none());
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(prompts.load(Ordering::SeqCst), 0);
    let mut updates = driver
        .prompt("new operator message".into(), vec![], Reach::Workspace)
        .await;
    while let Some(update) = updates.recv().await {
        if let Update::Message { text, .. } = update {
            assert!(!text.contains("private-auth"));
        }
    }
    assert_eq!(prompts.load(Ordering::SeqCst), 1);
    driver.invalidate();
    agent.abort();
}

#[tokio::test]
async fn auth_failure_cancel_disconnect_and_revocation_settle_without_raw_errors() {
    for finish in ["fail", "cancel", "disconnect", "revoke"] {
        let held = room(&format!("auth-{finish}"));
        let driver = Arc::new(ChildAgent::new(
            scratch(finish),
            "cursor".into(),
            "fixture".into(),
            TeammateTools::new(&held, "ada"),
        ));
        let agent = tokio::spawn(fixture(
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        ));
        driver
            .handshake(&persona(&scratch_cwd(), vec![]), client_transport())
            .await
            .unwrap();
        let owner = CancellationToken::new();
        let id = driver
            .clone()
            .auth_start(
                if finish == "fail" { "fail" } else { "wait" },
                owner.clone(),
            )
            .await
            .unwrap();
        if finish != "fail" {
            assert!(
                driver
                    .clone()
                    .auth_start("login", CancellationToken::new())
                    .await
                    .is_err()
            );
            match finish {
                "cancel" => driver.auth_cancel(&id).unwrap(),
                "disconnect" => owner.cancel(),
                "revoke" => driver.invalidate(),
                _ => unreachable!(),
            }
        }
        let status = finished(&driver, &id).await;
        assert_eq!(status.state, "failed");
        assert!(!status.error.unwrap().contains("private-auth-error"));
        assert!(lock(&driver.live.connection).is_none());
        agent.abort();
    }
}

#[test]
fn auth_prompt_failures_only_expose_safe_initialized_descriptors() {
    let live = Live::default();
    *lock(&live.auth_methods) = vec![acp::AuthMethod::Terminal(
        acp::AuthMethodTerminal::new("login", "Sign in").args(vec!["do-not-persist".into()]),
    )];
    let notice = auth::failure(&live, "authentication required", "acp_prompt").notice();
    assert_eq!(notice.contains("signIn"), cfg!(unix));
    assert!(!notice.contains("do-not-persist"));
    assert!(
        !auth::failure(&live, "ordinary failure", "acp_prompt")
            .notice()
            .contains("signIn")
    );
}

#[test]
fn auth_legacy_terminal_metadata_is_not_an_agent_flow() {
    for marker in [
        serde_json::json!({"command": "never-execute"}),
        serde_json::Value::Null,
        serde_json::json!(false),
    ] {
        let method: acp::AuthMethod = serde_json::from_value(serde_json::json!({
            "id": "legacy", "name": "Legacy terminal",
            "_meta": {"terminal-auth": marker}
        }))
        .unwrap();
        assert!(
            matches!(method, acp::AuthMethod::Agent(_)),
            "missing type defaults to Agent"
        );
        let live = Live::default();
        *lock(&live.auth_methods) = vec![method];
        assert!(auth::descriptor(&live).is_none());
    }
    let live = Live::default();
    *lock(&live.auth_methods) = vec![
        serde_json::from_value(serde_json::json!({
            "id": "ordinary", "name": "Ordinary login", "_meta": {"other-extension": true}
        }))
        .unwrap(),
    ];
    assert!(auth::descriptor(&live).is_some());
}

#[tokio::test]
async fn auth_start_does_not_wait_for_a_busy_operation_gate() {
    let held = room("auth-busy");
    let driver = Arc::new(ChildAgent::new(
        scratch("auth-busy"),
        "cursor".into(),
        "fixture".into(),
        TeammateTools::new(&held, "ada"),
    ));
    let _gate = driver.operation_gate.lock().await;
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        driver.clone().auth_start("login", CancellationToken::new()),
    )
    .await;
    assert!(
        result
            .expect("start must not wait behind prompt initialization")
            .is_err()
    );
    assert!(lock(&driver.live.auth).is_none());
}

// Isolate PATH in a subprocess, not the shared test process. The only executable
// a retry can launch is this fake harness; no installed login CLI is involved.
#[cfg(unix)]
#[tokio::test]
async fn auth_cancel_then_retry_owns_replacement_initialization() {
    const CASE: &str = "HOTLINE_TEST_AUTH_PREFLIGHT_CASE";
    let Ok(case) = std::env::var(CASE) else {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("auth-preflight-bin");
        let script = root.join("cursor-agent");
        std::fs::write(&script, r#"#!/bin/sh
IFS= read -r request || exit 1
printf initialized > initialize-seen
case "$HOTLINE_TEST_AUTH_PREFLIGHT_CASE" in
  cancel|disconnect|revoke) while IFS= read -r request; do :; done; exit 0 ;;
esac
id=${request#*\"id\":}
id=${id%%,*}
id=${id%%\}*}
if [ "$HOTLINE_TEST_AUTH_PREFLIGHT_CASE" = error ]; then
  printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"private-initialize-error"}}\n' "$id"
  while IFS= read -r request; do :; done
  exit 0
fi
methods='[]'
if [ "$HOTLINE_TEST_AUTH_PREFLIGHT_CASE" = legacy ]; then
  methods='[{"id":"wait","name":"Legacy login","_meta":{"terminal-auth":{"command":"never-execute"}}}]'
fi
printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"authMethods":%s}}\n' "$id" "$methods"
while IFS= read -r request; do
  case "$request" in
    *authenticate*) printf called > authenticate-seen ;;
  esac
  id=${request#*\"id\":}
  id=${id%%,*}
  id=${id%%\}*}
  printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"replacement"}}\n' "$id"
done
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        for case in [
            "cancel",
            "disconnect",
            "revoke",
            "removed",
            "legacy",
            "error",
        ] {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "driver::acp::auth_tests::auth_cancel_then_retry_owns_replacement_initialization", "--nocapture"])
                .env(CASE, case).env("PATH", &root).kill_on_drop(true);
            let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
                .await
                .expect("isolated preflight test terminated")
                .unwrap();
            assert!(
                output.status.success(),
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
        return;
    };

    let root = scratch(&format!("auth-preflight-{case}"));
    let held = room(&format!("auth-preflight-{case}"));
    let driver = Arc::new(ChildAgent::new(
        root.clone(),
        "cursor".into(),
        "fixture".into(),
        TeammateTools::new(&held, "ada"),
    ));
    let agent = tokio::spawn(fixture(
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    ));
    driver
        .handshake(
            &persona(&root.to_string_lossy(), vec![]),
            client_transport(),
        )
        .await
        .unwrap();
    let first = driver
        .clone()
        .auth_start("wait", CancellationToken::new())
        .await
        .unwrap();
    driver.auth_cancel(&first).unwrap();
    assert_eq!(finished(&driver, &first).await.state, "failed");
    assert!(lock(&driver.live.connection).is_none());
    assert!(
        driver
            .clone()
            .auth_start("unknown", CancellationToken::new())
            .await
            .is_err()
    );
    assert!(
        !root.join("initialize-seen").exists(),
        "unknown ids do not restart the harness"
    );

    let owner = CancellationToken::new();
    let retry = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        driver.clone().auth_start("wait", owner.clone()),
    )
    .await
    .expect("retry returns an attempt before replacement initialize completes")
    .unwrap();
    assert_ne!(retry, first);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !root.join("initialize-seen").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("replacement harness received initialize");
    match case.as_str() {
        "cancel" => driver.auth_cancel(&retry).unwrap(),
        "disconnect" => owner.cancel(),
        "revoke" => driver.invalidate(),
        _ => {}
    }
    let status = finished(&driver, &retry).await;
    assert_eq!(status.state, "failed");
    let error = status.error.unwrap();
    assert!(!error.contains("private-initialize-error"));
    if case == "removed" || case == "legacy" {
        assert!(
            error.contains("not offered by the current harness"),
            "{error}"
        );
    }
    assert!(lock(&driver.live.connection).is_none());
    assert!(
        lock(&driver.child).is_none(),
        "replacement child was reaped"
    );
    assert!(
        !root.join("authenticate-seen").exists(),
        "replacement methods are revalidated before authenticate"
    );
    driver.invalidate();
    agent.abort();
    std::fs::remove_dir_all(root).unwrap();
}
