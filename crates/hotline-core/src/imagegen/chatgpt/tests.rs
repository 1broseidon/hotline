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
