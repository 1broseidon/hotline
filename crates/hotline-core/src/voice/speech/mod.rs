//! The speech seam: `transcribe(clip)` and `speak(text)`, over whichever
//! connected provider the owner has.
//!
//! Most providers share the OpenAI audio shape, so one adapter covers them
//! ([`OpenAiShape`]); Google's Gemini has its own ([`Google`]). An adapter is
//! built for one job, listening or speaking, because the wire reports the
//! model behind each: the other job answers [`SpeechError::WrongJob`].
//!
//! Every call is timed and logged with the provider and model, as when the
//! response headers arrived, when its first byte of body did, and when it was
//! whole: whole-clip timings picked the wrong engine in Spark. One more line
//! runs from transcription to the first synthesized clip on a shared adapter
//! set ([`TurnClock`]); the desk separately times its first published clip.

mod catalog;
mod clip;
mod google;
mod openai_shape;
mod providers;
mod wav;
mod xai;

pub use clip::{MIN_GOODBYE_MS, billable_ms, plausible_goodbye};
pub use google::Google;
pub use openai_shape::{AudioFormat, Endpoint, OpenAiShape};
pub use providers::{options, resolve, resolve_output};

use async_trait::async_trait;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// One playable piece of audio and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// One ordered piece of a spoken reply. The final marker lets a caller
/// publish immediately instead of holding a playable clip for lookahead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechChunk {
    pub clip: Clip,
    pub final_chunk: bool,
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
    /// A subscription login is missing, revoked or no longer accepted.
    SignInRequired { provider_id: String },
    /// The subscription cannot use the requested speech capability.
    Entitlement { provider_id: String, status: u16 },
    /// The adapter was built for the other job.
    WrongJob,
    /// A clip in a format the adapter does not take.
    UnsupportedFormat(String),
    /// There was nothing to say.
    NothingToSay,
    /// The call stopped, or its caller stopped accepting audio.
    Cancelled,
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
            SpeechError::SignInRequired { provider_id } => {
                let name = if provider_id == "xai-subscription" {
                    "Grok"
                } else {
                    provider_id
                };
                write!(f, "Sign in to {name} again to use voice.")
            }
            SpeechError::Entitlement {
                provider_id,
                status,
            } => {
                let name = if provider_id == "xai-subscription" {
                    "Grok"
                } else {
                    provider_id
                };
                write!(
                    f,
                    "Your {name} subscription cannot use voice (HTTP {status})."
                )
            }
            SpeechError::WrongJob => write!(f, "This voice was not set up for that."),
            SpeechError::UnsupportedFormat(mime) => {
                write!(f, "Audio of type {mime} is not one Hotline can transcribe.")
            }
            SpeechError::NothingToSay => write!(f, "There was nothing to say."),
            SpeechError::Cancelled => write!(f, "The voice call was cancelled."),
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
    /// Subscription adapters do not incur an additional metered speech charge.
    fn is_subscription(&self) -> bool {
        false
    }
    /// Primary output advertised at call start; every clip still names its type.
    fn output_mime(&self) -> &str {
        "audio/wav"
    }
    /// The words in one clip (`audio/wav` or `audio/mp4`); empty when nobody spoke.
    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError>;
    /// Whether this listener accepts live mono PCM16 instead of a finished clip.
    fn supports_live_input(&self) -> bool {
        false
    }
    /// PCM16 little-endian mono frames. Closing `input` finalizes the utterance;
    /// dropping this future cancels the provider connection and pending work.
    async fn transcribe_live(
        &self,
        _input: tokio::sync::mpsc::Receiver<Vec<u8>>,
        _sample_rate: u32,
    ) -> Result<String, SpeechError> {
        Err(SpeechError::UnsupportedFormat("live PCM".into()))
    }
    /// One sentence to one playable clip.
    async fn speak(&self, text: &str) -> Result<Clip, SpeechError>;
    /// Ordered, independently playable clips. A bounded `output` applies
    /// backpressure; dropping its receiver or this future cancels synthesis.
    /// Providers without streaming support retain their whole-clip behavior.
    async fn speak_chunks(
        &self,
        text: &str,
        output: tokio::sync::mpsc::Sender<SpeechChunk>,
    ) -> Result<(), SpeechError> {
        if output.is_closed() {
            return Err(SpeechError::Cancelled);
        }
        tokio::select! {
            _ = output.closed() => Err(SpeechError::Cancelled),
            clip = self.speak(text) => {
                output.send(SpeechChunk { clip: clip?, final_chunk: true }).await.map_err(|_| SpeechError::Cancelled)
            }
        }
    }
}

/// What the owner's connected providers give the desk: one adapter to hear,
/// one to speak, and a second voice to fall back on.
#[derive(Clone)]
pub struct SpeechSet {
    pub stt: Arc<dyn Speech>,
    pub tts: Arc<dyn Speech>,
    pub fallback_tts: Option<Arc<dyn Speech>>,
}

/// Speaking does not require a transcription provider when the device supplies text.
#[derive(Clone)]
pub struct SpeechOutput {
    pub tts: Arc<dyn Speech>,
    pub fallback_tts: Option<Arc<dyn Speech>>,
}

/// A finished provider call: the body, and what the provider called it.
pub(crate) struct Reply {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// How long a call has taken: until its response headers came, until the
/// first byte of its body did, and until it was whole. For a provider that
/// streams, the headers can be early and the sound late, and for one that does
/// not, all three land together; the difference is the point of logging them.
struct Timing {
    started: Instant,
    headers: Option<Duration>,
    first_byte: Option<Duration>,
}

impl Timing {
    fn start() -> Timing {
        Timing {
            started: Instant::now(),
            headers: None,
            first_byte: None,
        }
    }

    fn headers_arrived(&mut self) {
        self.headers.get_or_insert_with(|| self.started.elapsed());
    }

    /// Only the first call counts.
    fn first_byte_arrived(&mut self) {
        self.first_byte
            .get_or_insert_with(|| self.started.elapsed());
    }

    fn line(&self, job: &str, id: &SpeechId) -> String {
        let done = self.started.elapsed();
        // An empty body has no first byte; the answer itself was the first.
        let headers = self.headers.unwrap_or(done);
        let first_byte = self.first_byte.unwrap_or(done);
        format!(
            "[voice] {job} {id}: headers {}ms, first byte {}ms, done {}ms",
            headers.as_millis(),
            first_byte.as_millis(),
            done.as_millis()
        )
    }
}

/// The stopwatch from transcription to the first synthesized clip on one
/// adapter set. The ears start it
/// when they begin to hear an utterance and its voices stop it, so the time
/// between (the dispatcher, a teammate's handoff) is inside it without either
/// side knowing the other. An utterance with nothing in it, or one that fails,
/// answers nothing and stops it; one that is never answered lapses.
#[derive(Clone, Default)]
pub struct TurnClock(Arc<Mutex<Option<Instant>>>);

/// No answer to an utterance takes longer than the dispatcher's own timeout.
const TURN_LAPSES_AFTER: Duration = Duration::from_secs(90);

impl TurnClock {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// An utterance has arrived.
    pub(crate) fn heard(&self) {
        *self.lock() = Some(Instant::now());
    }

    /// The utterance was not one to answer: it was empty, or it failed.
    pub(crate) fn unanswered(&self, words: &Result<String, SpeechError>) {
        if words.as_ref().map_or(true, |words| words.trim().is_empty()) {
            *self.lock() = None;
        }
    }

    /// A clip is ready. If it answers an utterance, logs how long that took
    /// and returns it; a clip with no utterance behind it (a delivery) logs
    /// nothing.
    pub(crate) fn first_clip(&self, voice: &SpeechId) -> Option<Duration> {
        let waited = self.lock().take()?.elapsed();
        if waited > TURN_LAPSES_AFTER {
            return None;
        }
        eprintln!(
            "[voice] transcription to synthesized clip {voice}: {}ms",
            waited.as_millis()
        );
        Some(waited)
    }

    #[cfg(test)]
    pub(crate) fn waiting(&self) -> bool {
        self.lock().is_some()
    }
}

/// The client every adapter shares its shape of. A key must never follow a
/// redirect to another server, and a call that hangs is a call that failed.
pub(crate) fn http_client() -> Result<reqwest::Client, String> {
    // Authorization belongs to each request, so a process-wide pool can be
    // reused when each new call resolves fresh credentials and adapters.
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not prepare the speech connection.".to_string())
        })
        .clone()
}

/// Sends a request, reads the body up to `limit` bytes, and logs how long the
/// headers, the first byte and the whole answer took.
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
    timing.headers_arrived();
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

    /// The numbers in a timing line, in the order it prints them.
    fn milliseconds(line: &str) -> Vec<u128> {
        line.split(|c: char| !c.is_ascii_digit())
            .filter_map(|word| word.parse().ok())
            .collect()
    }

    #[test]
    fn the_headers_the_first_byte_and_the_whole_are_three_moments() {
        let mut timing = Timing::start();
        std::thread::sleep(Duration::from_millis(30));
        timing.headers_arrived();
        std::thread::sleep(Duration::from_millis(40));
        timing.first_byte_arrived();
        std::thread::sleep(Duration::from_millis(40));
        timing.first_byte_arrived();
        timing.headers_arrived();

        let line = timing.line("speak", &id());

        assert!(
            line.starts_with("[voice] speak openai/tts-1: headers "),
            "{line}"
        );
        let numbers = milliseconds(&line);
        let (headers, first_byte, done) = (
            numbers[numbers.len() - 3],
            numbers[numbers.len() - 2],
            numbers[numbers.len() - 1],
        );
        // Sleeps only promise a minimum, and a loaded CI runner oversleeps:
        // assert the floors and the order, never a ceiling.
        assert!(headers >= 30, "{line}");
        assert!(first_byte >= headers + 40, "{line}");
        assert!(done >= first_byte + 40, "{line}");
    }

    #[test]
    fn an_empty_answer_has_no_moments_before_its_end() {
        let timing = Timing::start();
        std::thread::sleep(Duration::from_millis(20));
        let numbers = milliseconds(&timing.line("transcribe", &id()));
        let (headers, first_byte, done) = (
            numbers[numbers.len() - 3],
            numbers[numbers.len() - 2],
            numbers[numbers.len() - 1],
        );
        assert_eq!((headers, first_byte), (done, done));
    }

    #[test]
    fn an_utterance_is_timed_to_the_first_clip_that_answers_it() {
        let clock = TurnClock::default();
        assert!(!clock.waiting());
        assert_eq!(
            clock.first_clip(&id()),
            None,
            "a delivery has no utterance behind it"
        );

        clock.heard();
        assert!(clock.waiting());
        std::thread::sleep(Duration::from_millis(25));
        let waited = clock.first_clip(&id()).expect("the utterance's answer");
        assert!(waited >= Duration::from_millis(25) && waited < Duration::from_secs(5));
        // Only the first clip counts; a second sentence of the same answer does not.
        assert!(!clock.waiting());
        assert_eq!(clock.first_clip(&id()), None);
    }

    #[test]
    fn an_utterance_with_nothing_to_answer_stops_the_clock() {
        let clock = TurnClock::default();
        for nothing in [
            Ok(String::new()),
            Ok("  \n".to_string()),
            Err(SpeechError::Unreachable {
                provider_id: "openai".into(),
            }),
        ] {
            clock.heard();
            clock.unanswered(&nothing);
            assert!(!clock.waiting(), "{nothing:?}");
        }
        clock.heard();
        clock.unanswered(&Ok("ask Mack".to_string()));
        assert!(clock.waiting());
    }

    #[test]
    fn a_new_utterance_restarts_the_clock_and_a_stale_one_is_not_reported() {
        let clock = TurnClock::default();
        *clock.lock() = Some(Instant::now() - TURN_LAPSES_AFTER - Duration::from_secs(1));
        assert_eq!(clock.first_clip(&id()), None);

        *clock.lock() = Some(Instant::now() - Duration::from_secs(30));
        clock.heard();
        assert!(clock.first_clip(&id()).unwrap() < Duration::from_secs(5));
    }
}
