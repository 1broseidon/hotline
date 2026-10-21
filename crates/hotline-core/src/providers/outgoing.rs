//! The transport under a model whose requests Hotline finishes on their way
//! out: signed with a login's live token, and mended where a provider reads
//! a body differently from how Rig writes it.

use super::rig_login::Login;
use bytes::Bytes;
use futures_util::StreamExt;
use rig::http_client::{
    self, BoxedStream, HttpClientExt, LazyBody, MultipartForm, ReqwestClient, StreamingResponse,
};
use std::collections::VecDeque;
use std::time::Duration;

/// Every request is finished afresh. With a login, the login's
/// authenticator refreshes the stored token when it has expired, and
/// whatever bearer the client was built with is replaced by the live one, so
/// a turn that outlives a token does not end in a 401; the account a ChatGPT
/// token belongs to goes with it. `mend` is the last word on the body.
#[derive(Clone)]
pub(crate) struct Outgoing {
    login: Option<Login>,
    mend: fn(Bytes) -> Bytes,
    mend_stream: fn(BoxedStream) -> BoxedStream,
    http: ReqwestClient,
}

// A login holds the live token, so Debug shows none of it.
impl std::fmt::Debug for Outgoing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outgoing").finish_non_exhaustive()
    }
}

impl Outgoing {
    /// Requests signed with `login`'s live token, their bodies mended.
    pub(crate) fn signed(login: Login, mend: fn(Bytes) -> Bytes) -> Result<Self, String> {
        Self::with(Some(login), mend)
    }

    /// Requests sent with the credential Rig put on them, their bodies
    /// mended.
    pub(crate) fn mended(mend: fn(Bytes) -> Bytes) -> Result<Self, String> {
        Self::with(None, mend)
    }

    fn with(login: Option<Login>, mend: fn(Bytes) -> Bytes) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // A credential must never follow a redirect to another server.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not prepare the provider connection.".to_string())?;
        Ok(Self {
            login,
            mend,
            mend_stream: std::convert::identity,
            http: ReqwestClient::from(http),
        })
    }

    /// A streamed reply read through `mend` on its way back to Rig.
    pub(crate) fn mending_streams(mut self, mend: fn(BoxedStream) -> BoxedStream) -> Self {
        self.mend_stream = mend;
        self
    }

    #[expect(
        clippy::result_large_err,
        reason = "Rig's transport error is what a transport fails with"
    )]
    async fn finished(
        &self,
        request: http::Request<Bytes>,
    ) -> http_client::Result<http::Request<Bytes>> {
        let (mut parts, body) = request.into_parts();
        if let Some(login) = &self.login {
            let context = login.context().await.map_err(refused)?;
            let token = context.api_key.expose();
            if token.trim().is_empty() {
                return Err(refused(
                    "The sign-in has no token. Sign in again under Settings → Providers.".into(),
                ));
            }
            let mut bearer = http::HeaderValue::from_str(&format!("Bearer {token}"))?;
            bearer.set_sensitive(true);
            parts.headers.insert(http::header::AUTHORIZATION, bearer);
            if let Some(account) = context.account_id.filter(|account| !account.is_empty()) {
                parts
                    .headers
                    .insert("ChatGPT-Account-Id", http::HeaderValue::from_str(&account)?);
            }
        }
        Ok(http::Request::from_parts(parts, (self.mend)(body)))
    }
}

fn refused(message: String) -> http_client::Error {
    http_client::Error::instance(std::io::Error::other(message))
}

impl HttpClientExt for Outgoing {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let this = self.clone();
        let request = request.map(Into::into);
        async move {
            let request = this.finished(request).await?;
            this.http.send(request).await
        }
    }

    fn send_multipart<U>(
        &self,
        _: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(refused("A model request is never multipart.".into())))
    }

    async fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> http_client::Result<StreamingResponse>
    where
        T: Into<Bytes> + Send,
    {
        let request = self.finished(request.map(Into::into)).await?;
        let (parts, body) = self.http.send_streaming(request).await?.into_parts();
        Ok(http::Response::from_parts(parts, (self.mend_stream)(body)))
    }
}

/// A Chat Completions stream that ends with `data: [DONE]` is a whole reply,
/// so one that never named a `finish_reason` gets `stop` just before its
/// `[DONE]`. Copilot ends an empty reply (a model with nothing to add after a
/// tool call) with one chunk and no finish reason, which Rig reads as a
/// stream cut short. A stream that ends without `[DONE]` passes exactly as it
/// came, so a real cut stays one; so does every byte of a stream that named
/// its finish.
pub(crate) fn done_is_stop(inner: BoxedStream) -> BoxedStream {
    let state = DoneIsStop {
        inner,
        pending: Vec::new(),
        finished: false,
        last: None,
        out: VecDeque::new(),
        ended: false,
    };
    Box::pin(futures_util::stream::unfold(
        state,
        |mut state| async move {
            loop {
                if let Some(item) = state.out.pop_front() {
                    return Some((item, state));
                }
                if state.ended {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(bytes)) => {
                        state.pending.extend_from_slice(&bytes);
                        let mut emitted = Vec::new();
                        while let Some(at) = state.pending.iter().position(|byte| *byte == b'\n') {
                            let line: Vec<u8> = state.pending.drain(..=at).collect();
                            state.see(&line, &mut emitted);
                        }
                        if !emitted.is_empty() {
                            state.out.push_back(Ok(Bytes::from(emitted)));
                        }
                    }
                    Some(Err(error)) => state.out.push_back(Err(error)),
                    None => {
                        state.ended = true;
                        if !state.pending.is_empty() {
                            let rest = std::mem::take(&mut state.pending);
                            let mut emitted = Vec::new();
                            state.see(&rest, &mut emitted);
                            state.out.push_back(Ok(Bytes::from(emitted)));
                        }
                    }
                }
            }
        },
    ))
}

struct DoneIsStop {
    inner: BoxedStream,
    /// Bytes of a line not yet ended.
    pending: Vec<u8>,
    /// Whether a chunk has named a finish reason.
    finished: bool,
    /// The last chunk, whose id and model the added `stop` carries.
    last: Option<serde_json::Value>,
    out: VecDeque<http_client::Result<Bytes>>,
    ended: bool,
}

impl DoneIsStop {
    /// Passes `line` on, after the added `stop` when it is a `[DONE]` that
    /// came before any finish reason.
    fn see(&mut self, line: &[u8], emitted: &mut Vec<u8>) {
        let text = String::from_utf8_lossy(line);
        if let Some(data) = text.trim().strip_prefix("data:").map(str::trim) {
            if data == "[DONE]" {
                if !self.finished {
                    emitted.extend_from_slice(self.stop().as_bytes());
                    self.finished = true;
                }
            } else if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                let named = chunk["choices"].as_array().is_some_and(|choices| {
                    choices
                        .iter()
                        .any(|choice| !choice["finish_reason"].is_null())
                });
                self.finished |= named;
                self.last = Some(chunk);
            }
        }
        emitted.extend_from_slice(line);
    }

    /// The `stop` the stream left out, as its own event.
    fn stop(&self) -> String {
        let mut chunk = self
            .last
            .clone()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        chunk["choices"] = serde_json::json!([{"index": 0, "delta": {}, "finish_reason": "stop"}]);
        if let Some(fields) = chunk.as_object_mut() {
            // Usage already went with the chunk it came on.
            fields.remove("usage");
        }
        format!("data: {chunk}\n\n")
    }
}

#[cfg(test)]
mod done_is_stop_tests {
    use super::*;

    #[expect(
        clippy::result_large_err,
        reason = "Rig's transport error is what a stream yields"
    )]
    async fn read(chunks: Vec<&'static str>) -> String {
        let chunks: Vec<http_client::Result<Bytes>> = chunks
            .into_iter()
            .map(|chunk| Ok(Bytes::from(chunk)))
            .collect();
        let inner: BoxedStream = Box::pin(futures_util::stream::iter(chunks));
        let mut out = Vec::new();
        let mut stream = done_is_stop(inner);
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        String::from_utf8(out).unwrap()
    }

    /// Copilot's whole reply when Claude had nothing to add after a tool:
    /// one chunk, no finish reason, then `[DONE]`.
    const EMPTY: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":null}}],\"created\":1792513343,\"id\":\"1161ce38\",\"usage\":{\"completion_tokens\":2,\"prompt_tokens\":11064,\"total_tokens\":11066},\"model\":\"claude-sonnet-4.6\"}\n\ndata: [DONE]\n\n";

    #[tokio::test]
    async fn an_empty_reply_that_says_done_gets_its_stop() {
        let out = read(vec![EMPTY]).await;
        let events: Vec<&str> = out
            .split("\n\n")
            .filter(|event| !event.is_empty())
            .collect();
        assert_eq!(events.len(), 3, "{out}");
        let stop: serde_json::Value =
            serde_json::from_str(events[1].strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(stop["choices"][0]["finish_reason"], "stop");
        assert_eq!(stop["id"], "1161ce38");
        assert_eq!(stop["model"], "claude-sonnet-4.6");
        assert!(stop.get("usage").is_none(), "usage is not counted twice");
        assert_eq!(events[2], "data: [DONE]");
    }

    #[tokio::test]
    async fn a_reply_that_named_its_finish_passes_byte_for_byte() {
        let whole = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        assert_eq!(read(vec![whole]).await, whole);
    }

    #[tokio::test]
    async fn a_stream_cut_before_done_stays_cut() {
        let cut = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hal\"}}]}\n\n";
        assert_eq!(read(vec![cut]).await, cut);
    }

    #[tokio::test]
    async fn lines_and_characters_split_across_chunks_are_whole() {
        let out = read(vec![
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"caf\u{c3}",
            "\u{a9}\"}}]}\n\nda",
            "ta: [DO",
            "NE]\n\n",
        ])
        .await;
        assert!(out.contains("\"finish_reason\":\"stop\""), "{out}");
        assert!(out.ends_with("data: [DONE]\n\n"), "{out}");
    }
}
