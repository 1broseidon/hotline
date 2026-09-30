use super::http::{self, check, decode, malformed, sniff};
use super::{Image, ImageError, ImageGen, ImageId, ImageRequest};
use crate::providers::chatgpt::AuthRecord;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) const PROVIDER_ID: &str = "openai-codex";
pub(super) const MODEL: &str = "gpt-image-2";
const BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct ChatGpt {
    token_dir: PathBuf,
    base_url: String,
    http: reqwest::Client,
}

impl ChatGpt {
    pub(super) fn new(token_dir: PathBuf) -> Result<Self, String> {
        Ok(Self {
            token_dir,
            base_url: BASE_URL.into(),
            http: http::client()?,
        })
    }

    #[cfg(test)]
    pub(super) fn at(token_dir: PathBuf, base_url: String) -> Self {
        Self {
            base_url,
            ..Self::new(token_dir).unwrap()
        }
    }

    async fn generate_with_auth(
        &self,
        request: &ImageRequest,
        before_send: &(dyn Fn() -> Result<(), ImageError> + Send + Sync),
        refresh: impl Future<Output = Result<AuthRecord, String>> + Send,
        refresh_timeout: Duration,
    ) -> Result<Image, ImageError> {
        if request.prompt.trim().is_empty() {
            return Err(ImageError::EmptyPrompt);
        }
        if request.references.len() > self.max_references() {
            return Err(ImageError::TooManyReferences {
                max: self.max_references(),
            });
        }
        request.references.iter().try_for_each(check)?;
        let started = Instant::now();
        let auth = tokio::time::timeout(refresh_timeout, refresh)
            .await
            .map_err(|_| ImageError::RefreshTimedOut)?
            .map_err(|_| ImageError::SignInRequired)?;
        let mut body = json!({
            "model": MODEL,
            "prompt": format!("{}\n\nRequested aspect ratio: {}.", request.prompt, request.aspect.ratio()),
            "background": if request.transparent { "transparent" } else { "opaque" },
            "quality": "auto",
            "size": "auto",
        });
        let operation = if request.references.is_empty() {
            "generations"
        } else {
            body["images"] = request
                .references
                .iter()
                .map(|reference| json!({"image_url": http::data_url(reference)}))
                .collect();
            "edits"
        };
        let mut outgoing = self
            .http
            .post(format!("{}/images/{operation}", self.base_url))
            .bearer_auth(
                auth.access_token
                    .filter(|token| !token.trim().is_empty())
                    .ok_or(ImageError::SignInRequired)?,
            )
            .header("originator", "codex_cli_rs")
            .json(&body);
        if let Some(account) = auth.account_id.filter(|account| !account.is_empty()) {
            outgoing = outgoing.header("ChatGPT-Account-Id", account);
        }
        before_send()?;
        let answer = http::send(PROVIDER_ID, outgoing).await?;
        let answer: Value = serde_json::from_slice(&answer).map_err(|_| malformed(PROVIDER_ID))?;
        let encoded = answer["data"][0]["b64_json"]
            .as_str()
            .ok_or_else(|| malformed(PROVIDER_ID))?;
        let bytes = decode(PROVIDER_ID, encoded)?;
        let mime = sniff(&bytes).ok_or_else(|| malformed(PROVIDER_ID))?;
        Ok(Image {
            mime: mime.into(),
            bytes,
            id: self.id(),
            transparent: answer["background"] == "transparent",
            cost_usd: None,
            millis: started.elapsed().as_millis() as u64,
        })
    }
}

#[async_trait]
impl ImageGen for ChatGpt {
    fn id(&self) -> ImageId {
        ImageId {
            provider_id: PROVIDER_ID.into(),
            model_id: MODEL.into(),
        }
    }

    fn subscription(&self) -> bool {
        true
    }

    fn transparent(&self) -> bool {
        true
    }

    fn max_references(&self) -> usize {
        5
    }

    fn estimate_usd(&self, _request: &ImageRequest) -> f64 {
        0.0
    }

    async fn generate(&self, request: &ImageRequest) -> Result<Image, ImageError> {
        self.generate_checked(request, &|| Ok(())).await
    }

    async fn generate_checked(
        &self,
        request: &ImageRequest,
        before_send: &(dyn Fn() -> Result<(), ImageError> + Send + Sync),
    ) -> Result<Image, ImageError> {
        self.generate_with_auth(
            request,
            before_send,
            crate::providers::chatgpt::refreshed_auth(&self.token_dir),
            REFRESH_TIMEOUT,
        )
        .await
    }
}

#[cfg(test)]
#[path = "chatgpt_tests.rs"]
mod tests;
