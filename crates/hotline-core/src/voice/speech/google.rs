//! Gemini speech. Listening is `generateContent` with the clip inline and a
//! plain request to transcribe it; speaking is `generateContent` on a Gemini
//! TTS model, which returns raw PCM that is wrapped as a WAV to play.

use super::{Clip, Speech, SpeechError, SpeechId, TurnClock, base_mime, call, http_client, wav};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

pub const PROVIDER_ID: &str = "google";
pub const BASE_URL: &str = "https://generativelanguage.googleapis.com";

const TRANSCRIPT_LIMIT: usize = 1 << 20;
/// Inline audio comes back inside JSON as base64, a third bigger than it is.
const AUDIO_LIMIT: usize = 30 << 20;
/// Gemini TTS answers in 24 kHz unless it says otherwise.
const DEFAULT_RATE: u32 = 24_000;

/// This is a request to transcribe, not a style for a voice to perform, so it
/// is the one instruction voice ever sends a Gemini model.
const TRANSCRIBE: &str = "Transcribe the speech in this audio exactly as spoken. \
    Reply with only the words, and with nothing at all if nobody is speaking.";

enum Job {
    Listen { model: String },
    Speak { model: String, voice: String },
}

pub struct Google {
    base_url: String,
    key: String,
    job: Job,
    http: reqwest::Client,
    clock: TurnClock,
}

impl Google {
    pub fn listener(base_url: &str, key: &str, model: &str) -> Result<Google, String> {
        Ok(Google {
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            job: Job::Listen {
                model: model.to_string(),
            },
            http: http_client()?,
            clock: TurnClock::default(),
        })
    }

    pub fn speaker(base_url: &str, key: &str, model: &str, voice: &str) -> Result<Google, String> {
        Ok(Google {
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            job: Job::Speak {
                model: model.to_string(),
                voice: voice.to_string(),
            },
            http: http_client()?,
            clock: TurnClock::default(),
        })
    }

    /// Times an utterance to its first clip with the adapters that share this clock.
    pub fn with_clock(mut self, clock: &TurnClock) -> Google {
        self.clock = clock.clone();
        self
    }

    /// The key travels in a header, never in the URL, so it cannot end up in
    /// a log line that prints one.
    fn generate(&self, model: &str, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .post(format!(
                "{}/v1beta/models/{model}:generateContent",
                self.base_url
            ))
            .header("x-goog-api-key", &self.key)
            .json(body)
    }

    fn malformed(&self) -> SpeechError {
        SpeechError::Malformed {
            provider_id: PROVIDER_ID.to_string(),
        }
    }
}

/// Gemini takes an AAC file in an MP4 container as `audio/m4a`.
fn gemini_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "audio/wav" => Some("audio/wav"),
        "audio/mp4" => Some("audio/m4a"),
        _ => None,
    }
}

/// The sample rate in a mime type such as `audio/L16;codec=pcm;rate=24000`.
pub(super) fn pcm_rate(mime: &str) -> u32 {
    mime.split(';')
        .find_map(|parameter| parameter.trim().strip_prefix("rate="))
        .and_then(|rate| rate.parse().ok())
        .unwrap_or(DEFAULT_RATE)
}

#[async_trait]
impl Speech for Google {
    fn id(&self) -> SpeechId {
        let (model_id, voice) = match &self.job {
            Job::Listen { model } => (model.clone(), None),
            Job::Speak { model, voice } => (model.clone(), Some(voice.clone())),
        };
        SpeechId {
            provider_id: PROVIDER_ID.to_string(),
            model_id,
            voice,
        }
    }

    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError> {
        self.clock.heard();
        let words = self.hear(clip).await;
        self.clock.unanswered(&words);
        words
    }

    async fn speak(&self, text: &str) -> Result<Clip, SpeechError> {
        let clip = self.say(text).await;
        if clip.is_ok() {
            self.clock.first_clip(&self.id());
        }
        clip
    }
}

impl Google {
    async fn hear(&self, clip: Clip) -> Result<String, SpeechError> {
        let Job::Listen { model } = &self.job else {
            return Err(SpeechError::WrongJob);
        };
        let Some(mime) = gemini_mime(&base_mime(&clip.mime)) else {
            return Err(SpeechError::UnsupportedFormat(clip.mime));
        };
        let body = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {"text": TRANSCRIBE},
                    {"inlineData": {"mimeType": mime, "data": STANDARD.encode(&clip.bytes)}},
                ],
            }],
            "generationConfig": {"temperature": 0},
        });
        let reply = call(
            self.generate(model, &body),
            &self.id(),
            "transcribe",
            TRANSCRIPT_LIMIT,
        )
        .await?;
        let answer: Value = serde_json::from_slice(&reply.bytes).map_err(|_| self.malformed())?;
        // No candidate at all is Gemini declining to answer; a candidate
        // with no text is a clip with nobody in it.
        let candidates = answer
            .get("candidates")
            .and_then(Value::as_array)
            .ok_or_else(|| self.malformed())?;
        let text: String = candidates
            .first()
            .and_then(|candidate| candidate.pointer("/content/parts")?.as_array())
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text")?.as_str())
            .collect();
        Ok(text.trim().to_string())
    }

    async fn say(&self, text: &str) -> Result<Clip, SpeechError> {
        let Job::Speak { model, voice } = &self.job else {
            return Err(SpeechError::WrongJob);
        };
        if text.trim().is_empty() {
            return Err(SpeechError::NothingToSay);
        }
        // The words and nothing else: Gemini TTS reads any style instruction
        // in the prompt aloud, and that was only caught by transcribing the
        // output.
        let body = json!({
            "contents": [{"role": "user", "parts": [{"text": text}]}],
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "speechConfig": {
                    "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": voice}},
                },
            },
        });
        let reply = call(
            self.generate(model, &body),
            &self.id(),
            "speak",
            AUDIO_LIMIT,
        )
        .await?;
        let answer: Value = serde_json::from_slice(&reply.bytes).map_err(|_| self.malformed())?;
        let audio = answer
            .pointer("/candidates/0/content/parts/0/inlineData")
            .ok_or_else(|| self.malformed())?;
        let data = audio
            .get("data")
            .and_then(Value::as_str)
            .and_then(|data| STANDARD.decode(data).ok())
            .filter(|data| !data.is_empty())
            .ok_or_else(|| self.malformed())?;
        let mime = audio
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("audio/L16");
        match base_mime(mime).as_str() {
            "audio/wav" | "audio/mpeg" => Ok(Clip {
                mime: base_mime(mime),
                bytes: data,
            }),
            kind if kind.starts_with("audio/l16") || kind.starts_with("audio/pcm") => Ok(Clip {
                mime: "audio/wav".to_string(),
                bytes: wav::pcm16_wav(&data, pcm_rate(mime)),
            }),
            _ => Err(self.malformed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_comes_from_the_mime_type_and_defaults_to_24k() {
        assert_eq!(pcm_rate("audio/L16;codec=pcm;rate=16000"), 16_000);
        assert_eq!(pcm_rate("audio/L16; rate=44100"), 44_100);
        assert_eq!(pcm_rate("audio/L16"), 24_000);
        assert_eq!(pcm_rate("audio/L16;rate=fast"), 24_000);
    }

    #[test]
    fn a_phone_clip_goes_to_gemini_as_m4a() {
        assert_eq!(gemini_mime("audio/wav"), Some("audio/wav"));
        assert_eq!(gemini_mime("audio/mp4"), Some("audio/m4a"));
        assert_eq!(gemini_mime("audio/webm"), None);
    }
}
