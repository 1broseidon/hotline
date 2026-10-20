//! Credential-scoped origins for real-provider harnesses. This module is
//! absent from shipping builds; no setting or wire command can redirect a
//! built-in provider. Unique fixture credentials keep parallel tests apart.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

static ORIGINS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

pub struct Origin(String);

impl Origin {
    /// `credential` is an Anthropic fixture key or a ChatGPT login directory.
    /// The guard must outlive every request using it.
    pub fn new(credential: impl Into<String>, origin: &str) -> Self {
        let url = url::Url::parse(origin).expect("fixture origin");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        let credential = credential.into();
        assert!(
            ORIGINS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(credential.clone(), origin.into())
                .is_none(),
            "each fixture owns its credential"
        );
        Self(credential)
    }
}

impl Drop for Origin {
    fn drop(&mut self) {
        ORIGINS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

pub(crate) fn origin(credential: &str) -> Option<String> {
    ORIGINS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(credential)
        .cloned()
}

use bytes::Bytes;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, ReqwestClient, StreamingResponse,
};

#[derive(Clone)]
pub(crate) struct AuthHttp {
    base: String,
    http: ReqwestClient,
}

impl AuthHttp {
    pub(crate) fn new(base: String) -> Self {
        Self {
            base,
            http: ReqwestClient::default(),
        }
    }

    fn moved<T>(&self, request: http::Request<T>) -> http::Request<T> {
        let (mut parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or("/", http::uri::PathAndQuery::as_str);
        parts.uri = format!("{}{path}", self.base)
            .parse()
            .expect("the stub's address and a request path make a URI");
        http::Request::from_parts(parts, body)
    }
}

impl HttpClientExt for AuthHttp {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        self.http.send(self.moved(request))
    }

    fn send_multipart<U>(
        &self,
        request: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        self.http.send_multipart(self.moved(request))
    }

    fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        self.http.send_streaming(self.moved(request))
    }
}
