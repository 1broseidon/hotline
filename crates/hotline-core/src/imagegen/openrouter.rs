//! OpenRouter's unified Image API: one request shape for every image model
//! it serves (`POST /api/v1/images`), with the exact cost in `usage.cost`.

use super::http::{self, check, data_url, decode, malformed, sniff};
use super::{Image, ImageError, ImageGen, ImageId, ImageRequest, Model};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Instant;

pub const PROVIDER_ID: &str = "openrouter";
pub const BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouter {
    base_url: String,
    key: String,
    model: Model,
    http: reqwest::Client,
}

impl OpenRouter {
    pub fn new(base_url: &str, key: &str, model: Model) -> Result<OpenRouter, String> {
        Ok(OpenRouter {
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            model,
            http: http::client()?,
        })
    }

    fn body(&self, request: &ImageRequest) -> Value {
        let mut body = json!({
            "model": self.model.id,
            "prompt": request.prompt,
            "aspect_ratio": request.aspect.ratio(),
            "n": 1,
        });
        if let Some(quality) = self.model.quality {
            body["quality"] = json!(quality);
        }
        if request.transparent && self.model.transparent {
            body["background"] = json!("transparent");
            body["output_format"] = json!("png");
        }
        if !request.references.is_empty() {
            body["input_references"] = request
                .references
                .iter()
                .map(|reference| json!({"type": "image_url", "image_url": {"url": data_url(reference)}}))
                .collect();
        }
        body
    }
}

#[async_trait]
impl ImageGen for OpenRouter {
    fn id(&self) -> ImageId {
        ImageId {
            provider_id: PROVIDER_ID.into(),
            model_id: self.model.id.clone(),
        }
    }

    fn transparent(&self) -> bool {
        self.model.transparent
    }

    fn max_references(&self) -> usize {
        self.model.max_references
    }

    fn estimate_usd(&self, request: &ImageRequest) -> f64 {
        self.model.estimate_usd(request)
    }

    async fn generate(&self, request: &ImageRequest) -> Result<Image, ImageError> {
        self.model.admit(request)?;
        request.references.iter().try_for_each(check)?;
        let started = Instant::now();
        let answer = http::send(
            PROVIDER_ID,
            self.http
                .post(format!("{}/images", self.base_url))
                .bearer_auth(&self.key)
                .header("X-Title", "Hotline")
                .json(&self.body(request)),
        )
        .await?;
        let answer: Value = serde_json::from_slice(&answer).map_err(|_| malformed(PROVIDER_ID))?;
        let data = answer["data"][0]["b64_json"]
            .as_str()
            .ok_or_else(|| malformed(PROVIDER_ID))?;
        let bytes = decode(PROVIDER_ID, data)?;
        let mime = sniff(&bytes).ok_or_else(|| malformed(PROVIDER_ID))?;
        Ok(Image {
            mime: mime.into(),
            bytes,
            id: self.id(),
            transparent: request.transparent && self.model.transparent,
            cost_usd: answer["usage"]["cost"]
                .as_f64()
                // Some models report a cost of exactly zero; nothing is free, so
                // that counts as not reported and the estimate is charged.
                .filter(|cost| cost.is_finite() && *cost > 0.0),
            millis: started.elapsed().as_millis() as u64,
        })
    }
}
