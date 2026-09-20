//! Handing a running computer a saved login, out of band.
//!
//! The computer already knows how to load cookies into its browser: the agent
//! calls `state login_load`, which reads `.hotline/logins/<name>.json` and
//! restores it. Cookie import reuses that, driven by the desk instead of the
//! agent — the operator picked the sites, so the desk uploads the file through
//! the viewer's file endpoint and calls the load itself. The agent takes no
//! step, and there is no new tool for it to call. The cookie values ride from
//! the desk to the container over its authenticated loopback port and are
//! written to a file inside the sandbox the operator granted; they never enter
//! the tape, the model, or a log.

use super::Ready;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Implementation};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Uploads the login and loads it into the computer's browser. `saved` is a
/// `SavedLogin` document as the container writes them: `{name, browser,
/// created_at, cookies, storage}`.
pub async fn deliver(ready: &Ready, name: &str, saved: &Value) -> Result<(), String> {
    upload(ready, name, saved).await?;
    load(ready, name).await
}

/// Writes the document to `.hotline/logins/<name>.json` through the viewer's
/// authenticated `POST /files`, the same endpoint the operator's file drops
/// use.
///
/// That endpoint also takes the bearer as a query parameter, because the
/// browser it was built for cannot set a header. The desk is our own HTTP
/// client, not a browser, so it presents the bearer as an `Authorization`
/// header instead, which keeps the token out of the URL: nothing that logs a
/// request line — a proxy, a shell history — ever sees it.
///
/// A computer older than the release that learned to read that header only
/// looks at the query, and answers 401. Rather than make cookie import wait
/// on the image, the one refusal is retried the old way. The retry is the
/// only path that puts the token in a URL, and it is the path that would
/// have done so anyway. It can go once the version floor is past the release
/// that reads the header.
async fn upload(ready: &Ready, name: &str, saved: &Value) -> Result<(), String> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let path = format!(".hotline/logins/{name}.json");
    let address = format!("{base}/files");
    let body = serde_json::to_vec(saved).map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let url = reqwest::Url::parse_with_params(&address, &[("path", path.as_str())])
        .map_err(|error| format!("Could not address the computer's file endpoint: {error}"))?;
    let response = client
        .post(url)
        .bearer_auth(&ready.token)
        .body(body.clone())
        .send()
        .await
        .map_err(|error| format!("Could not send the cookies to the computer: {error}"))?;
    if response.status().is_success() {
        return Ok(());
    }
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!(
            "The computer refused the cookies ({}).",
            response.status()
        ));
    }

    let url = reqwest::Url::parse_with_params(
        &address,
        &[("token", ready.token.as_str()), ("path", path.as_str())],
    )
    .map_err(|error| format!("Could not address the computer's file endpoint: {error}"))?;
    let response = client
        .post(url)
        .body(body)
        .send()
        .await
        .map_err(|error| format!("Could not send the cookies to the computer: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "The computer refused the cookies ({}).",
            response.status()
        ));
    }
    Ok(())
}

/// Calls `state login_load` on the computer's MCP endpoint, the same way the
/// desk fetches the guide, so the browser picks the cookies up now.
async fn load(ready: &Ready, name: &str) -> Result<(), String> {
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?,
        StreamableHttpClientTransportConfig::with_uri(ready.url.clone())
            .auth_header(ready.token.clone()),
    );
    let service = ClientInfo::new(
        Default::default(),
        Implementation::new("hotline", env!("CARGO_PKG_VERSION")),
    )
    .serve(transport)
    .await
    .map_err(|error| format!("The computer's endpoint did not open: {error}"))?;
    let answered = service
        .call_tool(
            CallToolRequestParams::new("state").with_arguments(
                json!({"action": "login_load", "name": name})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await;
    service.cancel().await.ok();
    let result =
        answered.map_err(|error| format!("The computer could not load the cookies: {error}"))?;
    if result.is_error == Some(true) {
        let text = result
            .content
            .iter()
            .find_map(|block| block.as_text().map(|text| text.text.clone()))
            .unwrap_or_default();
        return Err(format!("The computer could not load the cookies: {text}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, Uri, header};
    use axum::routing::post;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// How the computer on the other end reads the bearer.
    #[derive(Clone, Copy, PartialEq)]
    enum Reads {
        /// A release that knows the `Authorization` header.
        Header,
        /// A release from before that, which only looks at `?token=`.
        QueryOnly,
    }

    #[derive(Clone)]
    struct Fake {
        reads: Reads,
        token: String,
        /// The query string of every request, so a test can prove the token
        /// did or did not ride in one.
        seen: Arc<std::sync::Mutex<Vec<String>>>,
        tries: Arc<AtomicUsize>,
    }

    /// The value of one parameter in a raw query string. The only values
    /// these tests read are the token, which has nothing needing decoding.
    fn parameter(query: &str, key: &str) -> Option<String> {
        query.split('&').find_map(|pair| {
            pair.split_once('=')
                .filter(|(name, _)| *name == key)
                .map(|(_, value)| value.to_owned())
        })
    }

    async fn files(
        State(fake): State<Fake>,
        headers: HeaderMap,
        uri: Uri,
        body: String,
    ) -> StatusCode {
        fake.tries.fetch_add(1, Ordering::SeqCst);
        let query = uri.query().unwrap_or_default().to_owned();
        fake.seen.lock().unwrap().push(query.clone());
        // A document the stand-in refuses for its own reasons, to stand for
        // any refusal that is not about the bearer.
        if body.contains("\"refuse\"") {
            return StatusCode::BAD_REQUEST;
        }
        let by_header = fake.reads == Reads::Header
            && headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                == Some(fake.token.as_str());
        if by_header || parameter(&query, "token").as_deref() == Some(fake.token.as_str()) {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        }
    }

    /// Serves a stand-in `/files` and hands back the `Ready` pointing at it.
    async fn computer(reads: Reads) -> (Ready, Fake, tokio::task::JoinHandle<()>) {
        let fake = Fake {
            reads,
            token: "the-token".to_owned(),
            seen: Arc::new(std::sync::Mutex::new(Vec::new())),
            tries: Arc::new(AtomicUsize::new(0)),
        };
        let app = axum::Router::new()
            .route("/files", post(files))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let serving = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "the-token".to_owned(),
        };
        (ready, fake, serving)
    }

    #[tokio::test]
    async fn a_current_computer_is_handed_the_token_in_a_header_and_never_in_the_url() {
        let (ready, fake, serving) = computer(Reads::Header).await;
        upload(&ready, "import-chrome-Default", &json!({"cookies": []}))
            .await
            .expect("the upload is accepted");
        assert_eq!(
            fake.tries.load(Ordering::SeqCst),
            1,
            "the header is enough, so nothing is retried"
        );
        let seen = fake.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert!(
            !seen[0].contains("token"),
            "the token never appears in the URL: {}",
            seen[0]
        );
        assert!(seen[0].contains("path="), "{}", seen[0]);
        serving.abort();
    }

    #[tokio::test]
    async fn a_computer_from_before_the_header_still_takes_the_cookies() {
        let (ready, fake, serving) = computer(Reads::QueryOnly).await;
        upload(&ready, "import-firefox-default", &json!({"cookies": []}))
            .await
            .expect("the upload is accepted the old way");
        assert_eq!(
            fake.tries.load(Ordering::SeqCst),
            2,
            "the header is refused once, then the query carries it"
        );
        let seen = fake.seen.lock().unwrap().clone();
        assert!(
            !seen[0].contains("token"),
            "the header is tried first, with a clean URL: {}",
            seen[0]
        );
        assert!(
            seen[1].contains("token=the-token"),
            "only the retry puts the token in the URL: {}",
            seen[1]
        );
        serving.abort();
    }

    #[tokio::test]
    async fn a_refusal_that_is_not_about_the_bearer_is_not_retried() {
        let (ready, fake, serving) = computer(Reads::Header).await;
        // The stand-in answers 400 to this document, not 401.
        let refused = upload(&ready, "import-chrome-Default", &json!({"refuse": true}))
            .await
            .expect_err("a bad request is an error");
        assert!(refused.contains("400"), "{refused}");
        assert_eq!(
            fake.tries.load(Ordering::SeqCst),
            1,
            "only a 401 is worth trying the old way"
        );
        serving.abort();
    }
}
