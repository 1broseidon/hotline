//! Gemini's image models: `generateContent` with the words (and any
//! reference images inline) and `responseModalities: ["IMAGE"]`. The key
//! travels in `x-goog-api-key`, never the URL.

use super::http::{self, base64, check, decode, malformed, sniff};
use super::{Image, ImageError, ImageGen, ImageId, ImageRequest, Model};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Instant;

pub const PROVIDER_ID: &str = "google";
pub const BASE_URL: &str = "https://generativelanguage.googleapis.com";

pub struct Google {
    base_url: String,
    key: String,
    model: Model,
    http: reqwest::Client,
}

impl Google {
    pub fn new(base_url: &str, key: &str, model: Model) -> Result<Google, String> {
        Ok(Google {
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            model,
            http: http::client()?,
        })
    }

    fn body(&self, request: &ImageRequest) -> Value {
        let mut parts = vec![json!({"text": request.prompt})];
        parts.extend(request.references.iter().map(|reference| {
            json!({"inline_data": {"mime_type": reference.mime, "data": base64(&reference.bytes)}})
        }));
        json!({
            "contents": [{"parts": parts}],
            "generationConfig": {
                "responseModalities": ["IMAGE"],
                "imageConfig": {"aspectRatio": request.aspect.ratio()},
            },
        })
    }
}

#[async_trait]
impl ImageGen for Google {
    fn id(&self) -> ImageId {
        ImageId {
            provider_id: PROVIDER_ID.into(),
            model_id: self.model.id.clone(),
        }
    }

    fn transparent(&self) -> bool {
        false
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
                .post(format!(
                    "{}/v1beta/models/{}:generateContent",
                    self.base_url, self.model.id
                ))
                .header("x-goog-api-key", &self.key)
                .json(&self.body(request)),
        )
        .await?;
        let answer: Value = serde_json::from_slice(&answer).map_err(|_| malformed(PROVIDER_ID))?;
        // The picture is whichever part carries inline data; a model may add words.
        let data = answer["candidates"][0]["content"]["parts"]
            .as_array()
            .and_then(|parts| {
                parts.iter().find_map(|part| {
                    part.get("inlineData")
                        .or_else(|| part.get("inline_data"))
                        .and_then(|inline| inline["data"].as_str())
                })
            })
            .ok_or_else(|| malformed(PROVIDER_ID))?;
        let bytes = decode(PROVIDER_ID, data)?;
        let mime = sniff(&bytes).ok_or_else(|| malformed(PROVIDER_ID))?;
        Ok(Image {
            mime: mime.into(),
            bytes,
            id: self.id(),
            transparent: false,
            cost_usd: None,
            millis: started.elapsed().as_millis() as u64,
        })
    }
}
