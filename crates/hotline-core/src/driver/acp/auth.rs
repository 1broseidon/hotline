//! Only initialized method ids reach this module; executable descriptors stay here.
use super::*;
use crate::driver::auth::{Attempt, AuthStatus, SignIn, SignInMethod};
use tokio_util::sync::CancellationToken;

fn supported(method: &acp::AuthMethod) -> bool {
    match method {
        // Before typed terminal methods, some harnesses put executable login
        // descriptors in metadata. Missing `type` deserializes as Agent, but
        // that does not make these legacy commands an authenticate flow.
        acp::AuthMethod::Agent(method) => !method
            .meta
            .as_ref()
            .is_some_and(|meta| meta.contains_key("terminal-auth")),
        acp::AuthMethod::Terminal(_) => cfg!(unix),
        _ => false,
    }
}

fn current_method(live: &Live, method_id: &str) -> Result<acp::AuthMethod, String> {
    lock(&live.auth_methods)
        .iter()
        .find(|method| method.id().to_string() == method_id && supported(method))
        .cloned()
        .ok_or_else(|| "That sign-in method is not offered by the current harness.".into())
}

pub(super) fn descriptor(live: &Live) -> Option<SignIn> {
    let methods = lock(&live.auth_methods)
        .iter()
        .filter(|method| supported(method))
        .filter(|method| method.id().to_string().len() <= 256)
        .take(32)
        .map(|method| SignInMethod {
            id: clip(&method.id().to_string(), 256),
            name: clip(method.name(), 256),
            description: method.description().map(|s| clip(s, 1024)),
        })
        .collect::<Vec<_>>();
    (!methods.is_empty()).then(|| SignIn {
        harness_name: clip(&lock(&live.session).info.agent_name, 256),
        methods,
    })
}

pub(super) fn failure(
    live: &Live,
    text: &str,
    phase: &'static str,
) -> super::super::failure::Failure {
    let mut failure = super::super::failure::Failure::classify(text, None, phase);
    if failure.kind == super::super::failure::Kind::AgentAuth {
        failure.sign_in = descriptor(live);
    }
    failure
}

pub(super) fn attempt(agent: &ChildAgent, id: &str) -> Result<Arc<Attempt>, String> {
    agent.check_capability()?;
    lock(&agent.live.auth)
        .as_ref()
        .filter(|a| a.id == id)
        .cloned()
        .ok_or_else(|| "That sign-in is no longer available.".into())
}

pub(super) async fn start(
    agent: Arc<ChildAgent>,
    method_id: &str,
    owner: CancellationToken,
) -> Result<String, String> {
    // A prompt may itself be waiting on a stalled initialize. Do not hold the
    // desktop's request open behind it: the socket must still observe closure.
    let _gate = agent
        .operation_gate
        .try_lock()
        .map_err(|_| "This teammate is busy. Try signing in again when it is idle.".to_string())?;
    agent.check_capability()?;
    if owner.is_cancelled() {
        return Err("The desktop connection has closed.".into());
    }
    if lock(&agent.live.auth).as_ref().is_some_and(|a| a.running()) {
        return Err("A sign-in is already running for this teammate.".into());
    }
    if lock(&agent.live.updates).is_some() {
        return Err("Stop this teammate's turn before signing in.".into());
    }
    // Reject unknown ids locally before creating work, even while disconnected.
    // The replacement harness must advertise this id again before it is used.
    current_method(&agent.live, method_id)?;
    let method_id = method_id.to_owned();
    let attempt = Attempt::new(owner);
    let id = attempt.id.clone();
    *lock(&agent.live.auth) = Some(attempt.clone());
    lock(&agent.live.stderr).clear();
    lock(&agent.live.open).take();
    lock(&agent.live.tools).clear();
    let runner = agent.clone();
    tokio::spawn(async move {
        let revoke = async {
            loop {
                if runner.check_capability().is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        };
        let work = async {
            // Cancellation closes ACP. Reinitialization is part of this attempt,
            // so a replacement that never initializes is still cancellable.
            if lock(&runner.live.connection).is_none() {
                runner.restart_after_failure().await.map_err(|_| {
                    "The harness could not restart for sign-in. You can try again.".to_string()
                })?;
            }
            let method = current_method(&runner.live, &method_id)?;
            authenticate(&runner, method, attempt.clone()).await
        };
        tokio::pin!(work);
        let result = tokio::select! {
            result = &mut work => result,
            _ = attempt.cancel.cancelled() => Err("Sign-in cancelled.".into()),
            _ = attempt.owner.cancelled() => Err("Sign-in cancelled when the desktop disconnected.".into()),
            _ = revoke => Err("Sign-in stopped because this teammate was stopped or changed.".into()),
            _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => Err("Sign-in timed out. You can try again.".into()),
        };
        if result.is_err() {
            attempt.cancel.cancel();
            // Closing authenticate is the only protocol-independent cancellation.
            // Terminal work observes the same token and confirms process exit.
            let _ = runner.abandon_failed_session().await;
            runner.live.failed.store(true, Ordering::SeqCst);
        }
        // A terminal's blocking worker must be reaped even if the outer request
        // was cancelled. authenticate owns that wait through terminal_done.
        let terminal = lock(&runner.live.terminal_done).take();
        if let Some(task) = terminal {
            let _ = task.await;
        }
        lock(&runner.live.stderr).clear();
        runner
            .live
            .auth_succeeded
            .store(result.is_ok(), Ordering::SeqCst);
        attempt.finish(result);
        runner.live.publish_info();
    });
    Ok(id)
}

async fn authenticate(
    agent: &ChildAgent,
    method: acp::AuthMethod,
    attempt: Arc<Attempt>,
) -> Result<(), String> {
    match method {
        acp::AuthMethod::Agent(method) => {
            let connection = lock(&agent.live.connection)
                .clone()
                .ok_or("The harness is disconnected.")?;
            connection
                .send_request(acp::AuthenticateRequest::new(method.id))
                .block_task()
                .await
                .map_err(|_| "The harness could not sign in. You can try again.".to_string())?;
            agent.check_capability()?;
            // Authentication does not repeat a failed prompt or restore its failed
            // checkpoint. Open a fresh session on the authenticated connection.
            let mut persona = lock(&agent.persona)
                .clone()
                .ok_or("The harness has no launch configuration.")?;
            persona.session_checkpoints.clear();
            let capabilities = lock(&agent.live.session).info.capabilities;
            agent
                .open_session(&connection, &persona, capabilities)
                .await
                .map_err(|_| {
                    "Signed in, but the harness could not open a session. Try again.".to_string()
                })?;
            agent.adopt_disposition(&persona).await;
        }
        acp::AuthMethod::Terminal(method) => {
            let launch = lock(&agent.launch)
                .clone()
                .ok_or("The harness has no terminal launch configuration.")?;
            let cwd = lock(&agent.persona)
                .as_ref()
                .ok_or("The harness has no workspace.")?
                .cwd
                .clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let result = crate::driver::auth::terminal(launch, cwd, method, attempt).await;
                let _ = tx.send(result);
            });
            *lock(&agent.live.terminal_done) = Some(task);
            rx.await
                .map_err(|_| "The sign-in terminal stopped unexpectedly.".to_string())??;
            agent.check_capability()?;
            agent.restart_after_failure().await.map_err(|_| {
                "Signed in, but the harness could not restart. Try again.".to_string()
            })?;
            if lock(&agent.live.startup_failure).is_some() {
                return Err("The harness still requires sign-in. You can try again.".into());
            }
        }
        _ => return Err("This authentication method is not supported.".into()),
    }
    agent.check_capability()?;
    *lock(&agent.live.startup_failure) = None;
    agent.live.failed.store(false, Ordering::SeqCst);
    agent.live.briefed.store(false, Ordering::SeqCst);
    agent.live.publish_info();
    Ok(())
}

pub(super) fn poll(agent: &ChildAgent, id: &str) -> Result<AuthStatus, String> {
    Ok(attempt(agent, id)?.poll())
}
