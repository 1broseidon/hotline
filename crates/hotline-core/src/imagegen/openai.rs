//! OpenAI's own Image API, and any custom connection that serves the same
//! shape: `POST /images/generations` from words, `POST /images/edits`
//! (multipart) when there are reference images.

use super::http::{self, check, decode, malformed, sniff};
use super::{Aspect, Image, ImageError, ImageGen, ImageId, ImageRequest, Model};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Instant;

pub const PROVIDER_ID: &str = "openai";
pub const BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAi {
    provider_id: String,
    base_url: String,
    key: Option<String>,
    model: Model,
    http: reqwest::Client,
}

impl OpenAi {
    pub fn new(
        provider_id: &str,
        base_url: &str,
        key: Option<&str>,
        model: Model,
    ) -> Result<OpenAi, String> {
        Ok(OpenAi {
            provider_id: provider_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.map(str::to_string),
            model,
            http: http::client()?,
        })
    }

    /// The API takes three sizes; each aspect goes to the nearest.
    fn size(aspect: Aspect) -> &'static str {
        match aspect {
            Aspect::Square => "1024x1024",
            Aspect::Wide | Aspect::Landscape => "1536x1024",
            Aspect::Tall | Aspect::Portrait => "1024x1536",
        }
    }

    fn fields(&self, request: &ImageRequest) -> Vec<(&'static str, String)> {
        let mut fields = vec![
            ("model", self.model.id.clone()),
            ("prompt", request.prompt.clone()),
            ("size", Self::size(request.aspect).to_string()),
            ("n", "1".to_string()),
        ];
        if let Some(quality) = self.model.quality {
            fields.push(("quality", quality.to_string()));
        }
        if request.transparent && self.model.transparent {
            fields.push(("background", "transparent".to_string()));
            fields.push(("output_format", "png".to_string()));
        }
        fields
    }

    fn authorised(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }

    fn request(&self, request: &ImageRequest) -> reqwest::RequestBuilder {
        if request.references.is_empty() {
            let body: serde_json::Map<String, Value> = self
                .fields(request)
                .into_iter()
                .map(|(name, value)| {
                    let value = if name == "n" { json!(1) } else { json!(value) };
                    (name.to_string(), value)
                })
                .collect();
            return self.authorised(
                self.http
                    .post(format!("{}/images/generations", self.base_url))
                    .json(&body),
            );
        }
        let (boundary, body) = multipart(&self.fields(request), request);
        self.authorised(
            self.http
                .post(format!("{}/images/edits", self.base_url))
                .header(
                    "Content-Type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(body),
        )
    }
}

/// A multipart body by hand: the text fields, then each reference as an
/// `image[]` file. The boundary can't occur in base64-free binary by chance
/// often enough to matter, and a clash only makes the provider refuse.
fn multipart(fields: &[(&'static str, String)], request: &ImageRequest) -> (String, Vec<u8>) {
    let boundary = format!("hotline-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    for (index, reference) in request.references.iter().enumerate() {
        let extension = reference.mime.rsplit('/').next().unwrap_or("png");
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image[]\"; filename=\"reference-{index}.{extension}\"\r\nContent-Type: {}\r\n\r\n",
                reference.mime
            )
            .as_bytes(),
        );
        body.extend_from_slice(&reference.bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

#[async_trait]
impl ImageGen for OpenAi {
    fn id(&self) -> ImageId {
        ImageId {
            provider_id: self.provider_id.clone(),
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
        let answer = http::send(&self.provider_id, self.request(request)).await?;
        let answer: Value =
            serde_json::from_slice(&answer).map_err(|_| malformed(&self.provider_id))?;
        let data = answer["data"][0]["b64_json"]
            .as_str()
            .ok_or_else(|| malformed(&self.provider_id))?;
        let bytes = decode(&self.provider_id, data)?;
        let mime = sniff(&bytes).ok_or_else(|| malformed(&self.provider_id))?;
        Ok(Image {
            mime: mime.into(),
            bytes,
            id: self.id(),
            transparent: request.transparent && self.model.transparent,
            // OpenAI reports tokens, not dollars.
            cost_usd: None,
            millis: started.elapsed().as_millis() as u64,
        })
    }
}
