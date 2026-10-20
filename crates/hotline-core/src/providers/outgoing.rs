//! The transport under a model whose requests Hotline finishes on their way
//! out: signed with a login's live token, and mended where a provider reads
//! a body differently from how Rig writes it.

use super::rig_login::Login;
use bytes::Bytes;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, ReqwestClient, StreamingResponse,
};
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
            http: ReqwestClient::from(http),
        })
    }

    async fn finished(
        &self,
        request: http::Request<Bytes>,
    ) -> http_client::Result<http::Request<Bytes>> {
        let (mut parts, body) = request.into_parts();
        if let Some(login) = &self.login {
            let context = login
                .context()
                .await
                .map_err(|error| refused(error.to_string()))?;
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
        self.http.send_streaming(request).await
    }
}
