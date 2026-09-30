//! The OpenAI audio shape: `POST {base}/audio/transcriptions` with a
//! multipart clip, `POST {base}/audio/speech` with a JSON sentence. OpenAI,
//! Groq, OpenRouter and Mistral (listening only) answer it, and so does a
//! custom `openai-compatible` connection that serves the `/audio` routes.

use super::{Clip, INPUT_TYPES, Speech, SpeechError, SpeechId, base_mime, call, http_client, wav};
use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

/// A transcript is a sentence; a spoken sentence is seconds of audio.
const TRANSCRIPT_LIMIT: usize = 1 << 20;
const AUDIO_LIMIT: usize = 20 << 20;

/// Where an OpenAI-shaped provider is and how to sign in to it. A custom
/// connection may be keyless.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub provider_id: String,
    /// The root the `/audio` routes hang from, without a trailing slash.
    pub base_url: String,
    pub key: Option<String>,
}

/// What a speaking adapter asks the provider to send back. Wav plays without
/// decoding; MP3 is what a server we know nothing about is most likely to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
}

impl AudioFormat {
    fn api_name(self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Mp3 => "audio/mpeg",
        }
    }
}

enum Job {
    Listen {
        model: String,
    },
    Speak {
        model: String,
        voice: String,
        format: AudioFormat,
        /// Some voices take only so many characters a request.
        max_chars: usize,
    },
}

pub struct OpenAiShape {
    endpoint: Endpoint,
    job: Job,
    http: reqwest::Client,
}

impl OpenAiShape {
    pub fn listener(endpoint: Endpoint, model: &str) -> Result<OpenAiShape, String> {
        Ok(OpenAiShape {
            endpoint,
            job: Job::Listen {
                model: model.to_string(),
            },
            http: http_client()?,
        })
    }

    pub fn speaker(
        endpoint: Endpoint,
        model: &str,
        voice: &str,
        format: AudioFormat,
        max_chars: usize,
    ) -> Result<OpenAiShape, String> {
        Ok(OpenAiShape {
            endpoint,
            job: Job::Speak {
                model: model.to_string(),
                voice: voice.to_string(),
                format,
                max_chars,
            },
            http: http_client()?,
        })
    }

    fn post(&self, route: &str) -> reqwest::RequestBuilder {
        let request = self
            .http
            .post(format!("{}/audio/{route}", self.endpoint.base_url));
        match &self.endpoint.key {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    fn malformed(&self) -> SpeechError {
        SpeechError::Malformed {
            provider_id: self.endpoint.provider_id.clone(),
        }
    }

    async fn speak_piece(
        &self,
        text: &str,
        model: &str,
        voice: &str,
        format: AudioFormat,
    ) -> Result<Clip, SpeechError> {
        // Bare text and nothing else: a style instruction is spoken aloud by
        // some engines, and the ones that would obey it are not the point.
        let request = self.post("speech").json(&json!({
            "model": model,
            "input": text,
            "voice": voice,
            "response_format": format.api_name(),
        }));
        let reply = call(request, &self.id(), "speak", AUDIO_LIMIT).await?;
        if reply.bytes.is_empty() {
            return Err(self.malformed());
        }
        Ok(Clip {
            mime: clip_mime(reply.content_type.as_deref(), format).to_string(),
            bytes: reply.bytes,
        })
    }
}

/// What the provider says it sent, if it is one of the two we ask for; else
/// what we asked for.
fn clip_mime(content_type: Option<&str>, asked: AudioFormat) -> &'static str {
    match content_type.map(base_mime).as_deref() {
        Some("audio/wav" | "audio/x-wav" | "audio/wave") => AudioFormat::Wav.mime(),
        Some("audio/mpeg" | "audio/mp3") => AudioFormat::Mp3.mime(),
        _ => asked.mime(),
    }
}

#[async_trait]
impl Speech for OpenAiShape {
    fn id(&self) -> SpeechId {
        let (model_id, voice) = match &self.job {
            Job::Listen { model } => (model.clone(), None),
            Job::Speak { model, voice, .. } => (model.clone(), Some(voice.clone())),
        };
        SpeechId {
            provider_id: self.endpoint.provider_id.clone(),
            model_id,
            voice,
        }
    }

    fn accepts(&self) -> &[&str] {
        INPUT_TYPES
    }

    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError> {
        let Job::Listen { model } = &self.job else {
            return Err(SpeechError::WrongJob);
        };
        // Providers tell a container by its file name.
        let mime = base_mime(&clip.mime);
        let file_name = match mime.as_str() {
            "audio/wav" => "clip.wav",
            "audio/mp4" => "clip.m4a",
            _ => return Err(SpeechError::UnsupportedFormat(clip.mime)),
        };
        let part = Part::bytes(clip.bytes)
            .file_name(file_name)
            .mime_str(&mime)
            .map_err(|_| self.malformed())?;
        // The model comes first and the file last, which every provider
        // accepts and a few insist on. No other field: a strict server
        // refuses one it does not know.
        let form = Form::new().text("model", model.clone()).part("file", part);
        let reply = call(
            self.post("transcriptions").multipart(form),
            &self.id(),
            "transcribe",
            TRANSCRIPT_LIMIT,
        )
        .await?;
        serde_json::from_slice::<Value>(&reply.bytes)
            .ok()
            .and_then(|body| {
                body.get("text")?
                    .as_str()
                    .map(|text| text.trim().to_string())
            })
            .ok_or_else(|| self.malformed())
    }

    async fn speak(&self, text: &str) -> Result<Clip, SpeechError> {
        let Job::Speak {
            model,
            voice,
            format,
            max_chars,
        } = &self.job
        else {
            return Err(SpeechError::WrongJob);
        };
        let pieces = split_for_speech(text, *max_chars);
        let mut clips = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            clips.push(self.speak_piece(piece, model, voice, *format).await?);
        }
        match clips.len() {
            0 => Err(SpeechError::NothingToSay),
            1 => Ok(clips.remove(0)),
            _ => join(clips, *format).ok_or_else(|| self.malformed()),
        }
    }
}

/// One clip from the pieces of a sentence spoken in parts.
fn join(clips: Vec<Clip>, format: AudioFormat) -> Option<Clip> {
    let bytes = match format {
        // MP3 frames stand alone, so the pieces play back to back as they are.
        AudioFormat::Mp3 => clips.into_iter().flat_map(|clip| clip.bytes).collect(),
        AudioFormat::Wav => {
            let wavs: Vec<Vec<u8>> = clips.into_iter().map(|clip| clip.bytes).collect();
            wav::join(&wavs)?
        }
    };
    Some(Clip {
        mime: format.mime().to_string(),
        bytes,
    })
}

/// The text in pieces of at most `max_chars`, cut at the end of a sentence if
/// there is one in reach, else a comma, else a space, else where it must be.
fn split_for_speech(text: &str, max_chars: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let window_end = rest
            .char_indices()
            .nth(max_chars)
            .map_or(rest.len(), |(at, _)| at);
        if window_end == rest.len() {
            pieces.push(rest.to_string());
            break;
        }
        let window = &rest[..window_end];
        let after = |marks: &[char]| {
            window
                .char_indices()
                .filter(|(at, mark)| {
                    marks.contains(mark)
                        && rest[at + mark.len_utf8()..].starts_with(char::is_whitespace)
                })
                .map(|(at, mark)| at + mark.len_utf8())
                .next_back()
        };
        let cut = after(&['.', '!', '?'])
            .or_else(|| after(&[',', ';', ':']))
            .or_else(|| window.rfind(char::is_whitespace).filter(|&at| at > 0))
            .unwrap_or(window_end);
        pieces.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_piece_and_empty_text_is_none() {
        assert_eq!(
            split_for_speech("Handing that to Mack.", 200),
            ["Handing that to Mack."]
        );
        assert!(split_for_speech("   ", 200).is_empty());
    }

    #[test]
    fn long_text_is_cut_at_sentence_ends_first() {
        let text = "Mack is on the failing PR. Clem finished the phone side. Nothing else is waiting on you.";
        let pieces = split_for_speech(text, 60);
        assert_eq!(
            pieces,
            [
                "Mack is on the failing PR. Clem finished the phone side.",
                "Nothing else is waiting on you."
            ]
        );
        assert!(pieces.iter().all(|piece| piece.chars().count() <= 60));
    }

    #[test]
    fn a_sentence_too_long_for_one_piece_is_cut_at_a_comma_then_a_space() {
        let pieces = split_for_speech("one two three, four five six seven eight nine ten", 20);
        assert_eq!(
            pieces,
            ["one two three,", "four five six seven", "eight nine ten"]
        );
        let pieces = split_for_speech("aaaaaaaaaa bbbbbbbbbb cccccccccc", 15);
        assert_eq!(pieces, ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"]);
    }

    #[test]
    fn a_word_longer_than_the_limit_is_cut_where_it_must_be_on_a_character() {
        let pieces = split_for_speech("ééééééééé", 4);
        assert_eq!(pieces, ["éééé", "éééé", "é"]);
    }

    #[test]
    fn the_clip_is_what_the_provider_says_it_is_when_that_is_a_kind_we_know() {
        assert_eq!(
            clip_mime(Some("audio/mpeg"), AudioFormat::Wav),
            "audio/mpeg"
        );
        assert_eq!(
            clip_mime(Some("audio/x-wav; charset=binary"), AudioFormat::Mp3),
            "audio/wav"
        );
        assert_eq!(
            clip_mime(Some("application/octet-stream"), AudioFormat::Wav),
            "audio/wav"
        );
        assert_eq!(clip_mime(None, AudioFormat::Mp3), "audio/mpeg");
    }
}
