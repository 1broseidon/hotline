//! The speech seam: `transcribe(clip)` and `speak(text)`, over whichever
//! connected provider the owner has.
//!
//! Most providers share the OpenAI audio shape, so one adapter covers them
//! ([`OpenAiShape`]); Google's Gemini has its own ([`Google`]). An adapter is
//! built for one job, listening or speaking, because the wire reports the
//! model behind each: the other job answers [`SpeechError::WrongJob`].
//!
//! Every call is timed, and the time to first byte is logged with the
//! provider and model: whole-clip timings picked the wrong engine in Spark.

mod google;
mod openai_shape;
mod providers;
mod wav;

pub use google::Google;
pub use openai_shape::{AudioFormat, Endpoint, OpenAiShape};
pub use providers::resolve;

use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The input types every adapter takes: the phone records AAC in an MP4
/// container, the window records 16 kHz mono PCM16 WAV.
const INPUT_TYPES: &[&str] = &["audio/wav", "audio/mp4"];

/// One playable piece of audio and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Which provider, model and voice an adapter speaks with. A listening
/// adapter has no voice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechId {
    pub provider_id: String,
    pub model_id: String,
    pub voice: Option<String>,
}

impl fmt::Display for SpeechId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider_id, self.model_id)
    }
}

/// Why a call did not produce words or sound. None of these carry a response
/// body: a provider's error text can echo what was said.
#[derive(Debug, PartialEq, Eq)]
pub enum SpeechError {
    /// The adapter was built for the other job.
    WrongJob,
    /// A clip in a format the adapter does not take.
    UnsupportedFormat(String),
    /// There was nothing to say.
    NothingToSay,
    /// The provider could not be reached, or did not answer in time.
    Unreachable { provider_id: String },
    /// The provider answered with an error status.
    Refused { provider_id: String, status: u16 },
    /// The provider answered, but not with anything usable.
    Malformed { provider_id: String },
}

impl fmt::Display for SpeechError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpeechError::WrongJob => write!(f, "This voice was not set up for that."),
            SpeechError::UnsupportedFormat(mime) => {
                write!(f, "Audio of type {mime} is not one Hotline can transcribe.")
            }
            SpeechError::NothingToSay => write!(f, "There was nothing to say."),
            SpeechError::Unreachable { provider_id } => {
                write!(f, "{provider_id} could not be reached.")
            }
            SpeechError::Refused {
                provider_id,
                status,
            } => write!(f, "{provider_id} refused the request (HTTP {status})."),
            SpeechError::Malformed { provider_id } => {
                write!(f, "{provider_id} answered with something unusable.")
            }
        }
    }
}

impl std::error::Error for SpeechError {}

#[async_trait]
pub trait Speech: Send + Sync {
    /// Provider, model and voice.
    fn id(&self) -> SpeechId;
    /// Input mime types `transcribe` takes.
    fn accepts(&self) -> &[&str];
    /// The words in one clip; empty when nobody spoke.
    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError>;
    /// One sentence to one playable clip.
    async fn speak(&self, text: &str) -> Result<Clip, SpeechError>;
}

/// What the owner's connected providers give the desk: one adapter to hear,
/// one to speak, and a second voice to fall back on.
#[derive(Clone)]
pub struct SpeechSet {
    pub stt: Arc<dyn Speech>,
    pub tts: Arc<dyn Speech>,
    pub fallback_tts: Option<Arc<dyn Speech>>,
}

impl SpeechSet {
    /// Speaks a sentence, and if the voice fails tries once on the fallback,
    /// so a provider failing never means silence.
    pub async fn speak(&self, text: &str) -> Result<Clip, SpeechError> {
        let error = match self.tts.speak(text).await {
            Ok(clip) => return Ok(clip),
            Err(error) => error,
        };
        let Some(fallback) = &self.fallback_tts else {
            return Err(error);
        };
        eprintln!(
            "[voice] {} could not speak ({error}); trying {}",
            self.tts.id(),
            fallback.id()
        );
        fallback.speak(text).await
    }
}

/// A finished provider call: the body, and what the provider called it.
pub(crate) struct Reply {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// How long a call has taken, and how long its first byte took. Time to
/// first sound is the number that matters for a voice, not the time to a
/// finished clip.
struct Timing {
    started: Instant,
    first_byte: Option<Duration>,
}

impl Timing {
    fn start() -> Timing {
        Timing {
            started: Instant::now(),
            first_byte: None,
        }
    }

    /// Only the first call counts.
    fn first_byte_arrived(&mut self) {
        self.first_byte
            .get_or_insert_with(|| self.started.elapsed());
    }

    fn line(&self, job: &str, id: &SpeechId) -> String {
        let done = self.started.elapsed();
        // An empty body has no first byte; the answer itself was the first.
        let first_byte = self.first_byte.unwrap_or(done);
        format!(
            "[voice] {job} {id}: first byte {}ms, done {}ms",
            first_byte.as_millis(),
            done.as_millis()
        )
    }
}

/// The client every adapter shares its shape of. A key must never follow a
/// redirect to another server, and a call that hangs is a call that failed.
pub(crate) fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare the speech connection.".to_string())
}

/// Sends a request, reads the body up to `limit` bytes, and logs how long the
/// first byte and the whole answer took.
pub(crate) async fn call(
    request: reqwest::RequestBuilder,
    id: &SpeechId,
    job: &str,
    limit: usize,
) -> Result<Reply, SpeechError> {
    let unreachable = || SpeechError::Unreachable {
        provider_id: id.provider_id.clone(),
    };
    let mut timing = Timing::start();
    let mut response = request.send().await.map_err(|_| unreachable())?;
    let status = response.status();
    if !status.is_success() {
        eprintln!(
            "[voice] {job} {id}: HTTP {} after {}ms",
            status.as_u16(),
            timing.started.elapsed().as_millis()
        );
        return Err(SpeechError::Refused {
            provider_id: id.provider_id.clone(),
            status: status.as_u16(),
        });
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unreachable())? {
        timing.first_byte_arrived();
        if bytes.len() + chunk.len() > limit {
            return Err(SpeechError::Malformed {
                provider_id: id.provider_id.clone(),
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    eprintln!("{}", timing.line(job, id));
    Ok(Reply {
        bytes,
        content_type,
    })
}

/// A mime type without its parameters or its case.
pub(crate) fn base_mime(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod timing_tests {
    use super::*;

    fn id() -> SpeechId {
        SpeechId {
            provider_id: "openai".into(),
            model_id: "tts-1".into(),
            voice: None,
        }
    }

    /// The two numbers in a timing line.
    fn milliseconds(line: &str) -> (u128, u128) {
        let numbers: Vec<u128> = line
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|word| word.parse().ok())
            .collect();
        (numbers[numbers.len() - 2], numbers[numbers.len() - 1])
    }

    #[test]
    fn the_first_byte_is_timed_from_the_request_and_only_the_first_counts() {
        let mut timing = Timing::start();
        std::thread::sleep(Duration::from_millis(40));
        timing.first_byte_arrived();
        std::thread::sleep(Duration::from_millis(40));
        timing.first_byte_arrived();

        let line = timing.line("speak", &id());

        assert!(
            line.starts_with("[voice] speak openai/tts-1: first byte "),
            "{line}"
        );
        let (first_byte, done) = milliseconds(&line);
        assert!((40..80).contains(&first_byte), "{line}");
        assert!(done >= 80, "{line}");
    }

    #[test]
    fn an_empty_answer_is_its_own_first_byte() {
        let timing = Timing::start();
        std::thread::sleep(Duration::from_millis(20));
        let (first_byte, done) = milliseconds(&timing.line("transcribe", &id()));
        assert_eq!(first_byte, done);
    }
}
