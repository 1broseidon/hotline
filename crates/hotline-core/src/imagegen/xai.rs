//! xAI's Imagine API, with an API key or a Grok subscription sign-in:
//! `POST /images/generations` from words, `POST /images/edits` when there
//! are reference images. Edits take JSON with the references as data URLs,
//! not the multipart form OpenAI's take.

use super::http::{self, check, decode, malformed, sniff};
use super::{Image, ImageError, ImageGen, ImageId, ImageRequest, Model};
use crate::credentials::CredentialFile;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const PROVIDER_ID: &str = "xai";
pub const BASE_URL: &str = "https://api.x.ai/v1";
const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) enum GrokAuth {
    Key(String),
    /// Draws on the subscription's limits: the bearer is the sign-in's, and
    /// nothing is charged to the spend ledger.
    Subscription(CredentialFile),
}

pub struct Grok {
    base_url: String,
    auth: GrokAuth,
    model: Model,
    http: reqwest::Client,
}

impl Grok {
    pub(super) fn new(base_url: &str, auth: GrokAuth, model: Model) -> Result<Grok, String> {
        Ok(Grok {
            base_url: base_url.trim_end_matches('/').to_string(),
            auth,
            model,
            http: http::client()?,
        })
    }

    /// Images come back as base64, not the temporary URL xAI answers with
    /// by default, so nothing is fetched from anywhere else.
    fn body(&self, request: &ImageRequest) -> Value {
        let mut body = json!({
            "model": self.model.id,
            "prompt": request.prompt,
            "n": 1,
            "response_format": "b64_json",
            "aspect_ratio": request.aspect.ratio(),
        });
        if let Some(quality) = self.model.quality {
            body["quality"] = json!(quality);
        }
        let image = |reference| json!({"url": http::data_url(reference), "type": "image_url"});
        match request.references.as_slice() {
            [] => {}
            [one] => body["image"] = image(one),
            many => body["images"] = many.iter().map(image).collect(),
        }
        body
    }

    async fn bearer(&self, rejected: Option<&str>) -> Result<String, ImageError> {
        match &self.auth {
            GrokAuth::Key(key) => Ok(key.clone()),
            GrokAuth::Subscription(tokens) => tokio::time::timeout(
                REFRESH_TIMEOUT,
                crate::providers::xai::bearer(tokens, rejected),
            )
            .await
            .map_err(|_| ImageError::RefreshTimedOut)?
            .map_err(|_| ImageError::SignInRequired),
        }
    }

    async fn send(&self, request: &ImageRequest, bearer: &str) -> Result<Vec<u8>, ImageError> {
        let operation = if request.references.is_empty() {
            "generations"
        } else {
            "edits"
        };
        http::send(
            PROVIDER_ID,
            self.http
                .post(format!("{}/images/{operation}", self.base_url))
                .bearer_auth(bearer)
                .json(&self.body(request)),
        )
        .await
    }
}

#[async_trait]
impl ImageGen for Grok {
    fn id(&self) -> ImageId {
        ImageId {
            provider_id: PROVIDER_ID.into(),
            model_id: self.model.id.clone(),
        }
    }

    fn subscription(&self) -> bool {
        matches!(self.auth, GrokAuth::Subscription(_))
    }

    fn transparent(&self) -> bool {
        self.model.transparent
    }

    fn max_references(&self) -> usize {
        self.model.max_references
    }

    fn estimate_usd(&self, request: &ImageRequest) -> f64 {
        if self.subscription() {
            0.0
        } else {
            self.model.estimate_usd(request)
        }
    }

    async fn generate(&self, request: &ImageRequest) -> Result<Image, ImageError> {
        self.generate_checked(request, &|| Ok(())).await
    }

    /// A subscription's bearer may need a refresh first, so the caller's
    /// authority is checked after that wait. A bearer refused as expired is
    /// refreshed and the request sent once more.
    async fn generate_checked(
        &self,
        request: &ImageRequest,
        before_send: &(dyn Fn() -> Result<(), ImageError> + Send + Sync),
    ) -> Result<Image, ImageError> {
        self.model.admit(request)?;
        request.references.iter().try_for_each(check)?;
        let started = Instant::now();
        let bearer = self.bearer(None).await?;
        before_send()?;
        let answer = match self.send(request, &bearer).await {
            Err(ImageError::Refused { status: 401, .. }) if self.subscription() => {
                let bearer = self.bearer(Some(&bearer)).await?;
                before_send()?;
                self.send(request, &bearer).await?
            }
            answer => answer?,
        };
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
            transparent: false,
            // xAI prices per picture and doesn't say what one cost.
            cost_usd: None,
            millis: started.elapsed().as_millis() as u64,
        })
    }
}
