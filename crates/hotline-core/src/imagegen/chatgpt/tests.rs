use super::*;
use crate::driver::CapabilityEpoch;
use axum::Router;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::oneshot;

struct Images {
    url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Images {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn images() -> Images {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let app = Router::new().fallback(move || {
        seen.fetch_add(1, Ordering::SeqCst);
        async {
            json!({
                "data": [{"b64_json": STANDARD.encode(b"\x89PNG\r\n\x1a\npicture")}]
            })
            .to_string()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Images {
        url,
        requests,
        task,
    }
}

fn auth() -> AuthRecord {
    AuthRecord {
        access_token: Some("test-token".into()),
        account_id: Some("test-account".into()),
    }
}

#[tokio::test]
async fn authority_is_checked_after_refresh_before_the_image_request() {
    let server = images().await;
    let tokens = tempfile::tempdir().unwrap();
    let adapter = ChatGpt::at(tokens.path().into(), server.url.clone());
    let request = ImageRequest {
        prompt: "a toad".into(),
        ..ImageRequest::default()
    };

    for revoke in [false, true] {
        let epoch = CapabilityEpoch::default();
        let lease = epoch.lease();
        let check = || lease.check().map_err(|_| ImageError::Revoked);
        let (started, waiting) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let refresh = async {
            started.send(()).unwrap();
            finished.await.unwrap();
            Ok(auth())
        };
        let (result, ()) = tokio::join!(
            adapter.generate_with_auth(&request, &check, refresh, REFRESH_TIMEOUT),
            async {
                waiting.await.unwrap();
                if revoke {
                    epoch.stop();
                }
                finish.send(()).unwrap();
            },
        );
        if revoke {
            assert_eq!(result.err().unwrap(), ImageError::Revoked);
        } else {
            assert!(result.is_ok());
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn refresh_timeout_is_retryable_and_never_dispatches_an_image() {
    let server = images().await;
    let tokens = tempfile::tempdir().unwrap();
    let adapter = ChatGpt::at(tokens.path().into(), server.url.clone());
    let request = ImageRequest {
        prompt: "a toad".into(),
        ..ImageRequest::default()
    };
    let checks = AtomicUsize::new(0);
    let check = || {
        checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    };
    let error = adapter
        .generate_with_auth(&request, &check, std::future::pending(), Duration::ZERO)
        .await
        .err()
        .unwrap();
    assert_eq!(error, ImageError::RefreshTimedOut);
    assert_eq!(
        error.to_string(),
        "The ChatGPT sign-in refresh timed out. Try again."
    );
    assert_eq!(checks.load(Ordering::SeqCst), 0);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);

    let invalid = adapter
        .generate_with_auth(
            &request,
            &check,
            std::future::ready(Err("private authentication error".into())),
            REFRESH_TIMEOUT,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(invalid, ImageError::SignInRequired);
    assert!(!invalid.to_string().contains("private authentication error"));
    assert_eq!(checks.load(Ordering::SeqCst), 0);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

/// Sign in with ChatGPT draws on the public Images API with its token and
/// nothing that names Codex or an account; a refusal there says it is this
/// sign-in that cannot draw, rather than asking for a new sign-in.
#[tokio::test]
async fn an_agent_sign_in_draws_on_the_public_api_and_says_so_when_refused() {
    use axum::http::{HeaderMap, StatusCode, Uri};
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let refuse = Arc::new(AtomicUsize::new(0));
    let refusing = refuse.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap| {
        let tx = tx.clone();
        let refusing = refusing.clone();
        async move {
            tx.send((uri.path().to_string(), headers)).await.unwrap();
            if refusing.load(Ordering::SeqCst) > 0 {
                return (StatusCode::FORBIDDEN, String::new());
            }
            (
                StatusCode::OK,
                json!({"data": [{"b64_json": STANDARD.encode(b"\x89PNG\r\n\x1a\npicture")}]})
                    .to_string(),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tokens = tempfile::tempdir().unwrap();
    crate::providers::rig_login::agent::seed(tokens.path(), "agent-access").await;
    let adapter = ChatGpt::at(tokens.path().into(), url);
    assert!(adapter.agent);
    let request = ImageRequest {
        prompt: "a toad".into(),
        ..ImageRequest::default()
    };
    let agent_auth = || async {
        Ok(AuthRecord {
            access_token: Some("agent-access".into()),
            account_id: None,
        })
    };

    let image = adapter
        .generate_with_auth(&request, &|| Ok(()), agent_auth(), REFRESH_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(image.id.provider_id, PROVIDER_ID);
    let (path, headers) = rx.recv().await.unwrap();
    assert_eq!(path, "/images/generations");
    assert_eq!(headers["authorization"], "Bearer agent-access");
    assert!(headers.get("originator").is_none());
    assert!(headers.get("chatgpt-account-id").is_none());

    refuse.store(1, Ordering::SeqCst);
    let refused = adapter
        .generate_with_auth(&request, &|| Ok(()), agent_auth(), REFRESH_TIMEOUT)
        .await
        .unwrap_err();
    assert_eq!(refused, ImageError::NotForThisSignIn { status: 403 });
    assert!(refused.to_string().contains("device code"));
    assert_eq!(
        ChatGpt::new(tokens.path().into()).unwrap().base_url,
        "https://api.openai.com/v1"
    );
    server.abort();
}
