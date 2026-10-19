//! A provider on localhost: an HTTP server that records each request and
//! answers with a canned reply. The chain's own tests use it, and so do the
//! core's tests of the `web_search` tool, through the `test-support` feature.

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What one mock request looked like.
pub struct Seen {
    pub path: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// What a mock answers.
#[derive(Clone)]
pub struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    delay: Duration,
}

impl Reply {
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    /// One SSE frame carrying `rpc`, as the hosted MCP servers answer.
    pub fn sse(rpc: impl AsRef<str>) -> Self {
        Self {
            content_type: "text/event-stream",
            body: format!("event: message\ndata: {}\n\n", rpc.as_ref()),
            ..Self::json("")
        }
    }

    /// An MCP tool result whose one text block is `text`.
    pub fn tool_text(text: &str) -> String {
        json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":text}]}})
            .to_string()
    }

    pub fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            ..Self::json(body)
        }
    }

    pub fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

pub struct Mock {
    /// `http://127.0.0.1:port`
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Mock {
    /// `/mcp` on this server.
    pub fn mcp(&self) -> String {
        format!("{}/mcp", self.base)
    }

    pub fn only(&self) -> Seen {
        let mut seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "expected exactly one request");
        seen.remove(0)
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn mock(reply: Reply) -> Mock {
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let record = seen.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let record = record.clone();
        let reply = reply.clone();
        async move {
            record.lock().unwrap().push(Seen {
                path: uri.path_and_query().unwrap().to_string(),
                headers,
                body: bytes.to_vec(),
            });
            tokio::time::sleep(reply.delay).await;
            (
                StatusCode::from_u16(reply.status).unwrap(),
                [(header::CONTENT_TYPE, reply.content_type)],
                reply.body,
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Mock { base, seen, task }
}
