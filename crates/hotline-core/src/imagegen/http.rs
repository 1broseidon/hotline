//! What every adapter shares: one client, answers read with a ceiling, and
//! errors that name the provider and status and never repeat a body.

use super::{ImageError, Reference};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::StreamExt as _;
use std::time::Duration;

/// A picture comes back as base64 inside JSON, a third bigger than it is.
/// The largest a 4K PNG plausibly weighs, and then some.
pub(super) const ANSWER_LIMIT: usize = 48 << 20;
/// What one reference image may weigh before it's sent.
pub(super) const REFERENCE_LIMIT: usize = 20 << 20;
/// Reference types every adapter can send.
pub(super) const REFERENCE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp"];

/// Image models take seconds to minutes (the slowest in the 30 Sep bake-off
/// took 96s), so the wait is long; redirects are off so a key never follows
/// one somewhere else.
pub(super) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(150))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare the image connection.".to_string())
}

/// Sends a request and reads a successful answer, up to [`ANSWER_LIMIT`].
pub(super) async fn send(
    provider_id: &str,
    request: reqwest::RequestBuilder,
) -> Result<Vec<u8>, ImageError> {
    let unreachable = || ImageError::Unreachable {
        provider_id: provider_id.to_string(),
    };
    let response = request.send().await.map_err(|_| unreachable())?;
    let status = response.status();
    if !status.is_success() {
        return Err(ImageError::Refused {
            provider_id: provider_id.to_string(),
            status: status.as_u16(),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length as usize > ANSWER_LIMIT)
    {
        return Err(malformed(provider_id));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| unreachable())?;
        if body.len() + chunk.len() > ANSWER_LIMIT {
            return Err(malformed(provider_id));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) fn malformed(provider_id: &str) -> ImageError {
    ImageError::Malformed {
        provider_id: provider_id.to_string(),
    }
}

pub(super) fn decode(provider_id: &str, data: &str) -> Result<Vec<u8>, ImageError> {
    let bytes = STANDARD
        .decode(data.trim())
        .map_err(|_| malformed(provider_id))?;
    if bytes.is_empty() {
        return Err(malformed(provider_id));
    }
    Ok(bytes)
}

/// The type the bytes say they are, which beats what a provider claims.
pub(super) fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes
        .iter()
        .take(512)
        .copied()
        .collect::<Vec<u8>>()
        .windows(4)
        .any(|window| window == b"<svg")
    {
        Some("image/svg+xml")
    } else {
        None
    }
}

/// A reference fit to send, or why not.
pub(super) fn check(reference: &Reference) -> Result<(), ImageError> {
    if !REFERENCE_TYPES.contains(&reference.mime.as_str())
        || reference.bytes.len() > REFERENCE_LIMIT
    {
        return Err(ImageError::UnsupportedReference(reference.mime.clone()));
    }
    Ok(())
}

pub(super) fn base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub(super) fn data_url(reference: &Reference) -> String {
    format!(
        "data:{};base64,{}",
        reference.mime,
        base64(&reference.bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_say_what_they_are() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><svg xmlns="),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"{\"error\":1}"), None);
    }

    #[test]
    fn only_pictures_of_a_sendable_size_are_references() {
        let fits = Reference {
            mime: "image/png".into(),
            bytes: vec![0; 10],
        };
        assert!(check(&fits).is_ok());
        let gif = Reference {
            mime: "image/gif".into(),
            bytes: vec![0; 10],
        };
        assert_eq!(
            check(&gif),
            Err(ImageError::UnsupportedReference("image/gif".into()))
        );
        let huge = Reference {
            mime: "image/png".into(),
            bytes: vec![0; REFERENCE_LIMIT + 1],
        };
        assert!(check(&huge).is_err());
    }
}
