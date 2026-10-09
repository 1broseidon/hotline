//! The desk's own ears: speech to text on the desk's machine, with a model
//! the owner downloaded (`install.rs`). Nothing said leaves the machine and
//! nothing is charged, so it is the provider `local` at a price of zero.
//!
//! The engine is sherpa-onnx, linked statically with onnxruntime, so it runs
//! wherever the desk does with nothing installed beside it. It runs three
//! kinds of model ([`Engine`]): NVIDIA's Parakeet transducers, OpenAI's
//! Whisper and Useful Sensors' Moonshine. A model on disk describes itself
//! (`model.json`), so hearing never needs the download catalogue: what is in
//! the directory is what can hear.
//!
//! A transducer listens for words it is given (`hotwords`): teammates' names
//! and the person's own words, which a small model otherwise spells as the
//! nearest common word ("Parakey" for Parakeet). The words go with each
//! utterance, so they change without reloading the model.
//!
//! One model is loaded at a time and kept while it is used, because loading
//! takes a quarter to half a second and the larger model holds about a
//! gigabyte; a model nobody has asked for in [`IDLE`] is let go.
//!
//! The engine is C++ under a C API, and an exception it throws cannot be
//! caught from Rust: the whole desk would stop. Two inputs throw. A damaged
//! model file, so each file is checked against the SHA-256 recorded when it
//! was unpacked from its verified archive, once per run, before it is
//! loaded. And audio too short to make one frame, so less than a tenth of a
//! second is heard as nothing without running the model. The engine also
//! ends the process outright on a few misuses, which this file never
//! makes: hotwords for a model that is not a transducer, a transducer
//! loaded for hotwords without its vocabulary file, or a hotword line in
//! the engine's own syntax.

mod install;

pub use install::{Installs, Model, catalogue};

use super::{Clip, Speech, SpeechError, SpeechId, TurnClock, wav};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    OfflineMoonshineModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineTransducerModelConfig, OfflineWhisperModelConfig,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub const PROVIDER_ID: &str = "local";
/// What the picker calls it. The desk's machine, said as the window says it:
/// a phone or a window on another computer hears through it too.
pub const PROVIDER_NAME: &str = "On the desk";

/// What kind of model a model is, which says what its files are and how
/// the engine runs it. Each is as sherpa-onnx exports it, quantized to
/// eight bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// A NeMo transducer: Parakeet. A model unpacked before there were
    /// other kinds has no engine in its manifest, and is one of these.
    #[default]
    Transducer,
    Whisper,
    Moonshine,
}

impl Engine {
    /// The model's files, as the desk keeps them.
    fn files(self) -> &'static [&'static str] {
        match self {
            Engine::Transducer => &[
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt",
            ],
            Engine::Whisper => &["encoder.int8.onnx", "decoder.int8.onnx", "tokens.txt"],
            Engine::Moonshine => &[
                "preprocess.onnx",
                "encode.int8.onnx",
                "uncached_decode.int8.onnx",
                "cached_decode.int8.onnx",
                "tokens.txt",
            ],
        }
    }

    /// The most a model hears at once. Whisper's engine keeps only the first
    /// thirty seconds of a clip, and Moonshine starts repeating itself on a
    /// long one, so either hears a long clip in pieces.
    fn longest_piece(self) -> Option<Duration> {
        match self {
            Engine::Transducer => None,
            Engine::Whisper | Engine::Moonshine => Some(Duration::from_secs(28)),
        }
    }
}

const MANIFEST: &str = "model.json";
/// The transducer's vocabulary scored for the engine's hotword encoder,
/// written beside the model when it loads ([`scored_vocabulary`]).
const VOCABULARY: &str = "bpe.vocab";

/// How strongly a transducer favours each piece of a word it listens for:
/// sherpa-onnx's own default. On the fixtures it took names from 5 of 18 to
/// 15 of 18 on Parakeet English; 2.0 began capitalising ordinary words that
/// start like a name ("Parka").
const HOTWORD_SCORE: f32 = 1.5;
/// The most words a stream listens for. A word's first piece is favoured
/// wherever a word could begin, and the engine does not take that back when
/// the rest of the word does not follow, so a long list capitalises words
/// that only begin like one: on the fixtures 16 cost nothing, 40 doubled the
/// stray capitals and 157 quadrupled them. Decoding time does not change.
const MAX_HOTWORDS: usize = 32;
/// The longest word or phrase listened for, in characters.
const MAX_HOTWORD_CHARS: usize = 40;

/// The longest clip the engine takes. Dictation sends at most half of this
/// at a time and a call's utterances are capped below it; the larger model
/// needs about two gigabytes for two minutes.
pub const MAX_SECONDS: usize = 60;
const RATE: usize = 16_000;

/// A loaded model nobody has used for this long is let go.
const IDLE: Duration = Duration::from_secs(300);

/// Where models live under the data directory.
pub fn models_dir(root: &Path) -> PathBuf {
    root.join("speech-models")
}

/// What a model says of itself on disk: what it is, and what each of its
/// files was when it came out of the verified archive.
#[derive(Serialize, Deserialize)]
struct Manifest {
    id: String,
    name: String,
    #[serde(default)]
    engine: Engine,
    files: Vec<FileHash>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileHash {
    name: String,
    bytes: u64,
    sha256: String,
}

/// A model on disk, ready to hear with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub dir: PathBuf,
    files: Vec<FileHash>,
}

/// The models on disk, in the catalogue's order (most accurate first), then
/// any the catalogue no longer lists. A directory without its manifest or one
/// of its files is not a model: a download that never finished leaves one.
pub fn installed(root: &Path) -> Vec<Installed> {
    let Ok(entries) = std::fs::read_dir(models_dir(root)) else {
        return Vec::new();
    };
    let mut found: Vec<Installed> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| read_model(&entry.path()))
        .collect();
    let order = catalogue();
    let rank = |id: &str| {
        order
            .iter()
            .position(|model| model.id == id)
            .unwrap_or(usize::MAX)
    };
    found.sort_by(|a, b| rank(&a.id).cmp(&rank(&b.id)).then(a.id.cmp(&b.id)));
    found
}

fn read_model(dir: &Path) -> Option<Installed> {
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
    // The directory is named for the model, so a manifest cannot claim another's place.
    let named = dir.file_name()?.to_str()? == manifest.id;
    // Every file is there at the size it was unpacked at; its bytes are checked before loading.
    let whole = manifest.engine.files().iter().all(|file| {
        let recorded = manifest.files.iter().find(|one| one.name == *file);
        let size = std::fs::metadata(dir.join(file))
            .ok()
            .filter(|meta| meta.is_file());
        matches!((recorded, size), (Some(recorded), Some(size)) if recorded.bytes == size.len())
    });
    (named && whole).then(|| Installed {
        id: manifest.id,
        name: manifest.name,
        engine: manifest.engine,
        dir: dir.to_path_buf(),
        files: manifest.files,
    })
}

/// Model directories whose files were checked against their hashes this run.
static VERIFIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Whether every file of `model` is the file its archive held. Read once
/// per run: a second of hashing, against a damaged file stopping the desk.
fn verify(model: &Installed) -> Result<(), SpeechError> {
    if VERIFIED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&model.dir)
    {
        return Ok(());
    }
    let damaged = || {
        SpeechError::Engine(
            "The speech model on the desk is damaged. Remove it in Settings and download it again."
                .into(),
        )
    };
    for file in &model.files {
        let mut reader = std::fs::File::open(model.dir.join(&file.name)).map_err(|_| damaged())?;
        let (_, sha256) =
            install::hash_copy(&mut reader, &mut std::io::sink(), &CancellationToken::new())
                .map_err(|_| damaged())?;
        if sha256 != file.sha256 {
            return Err(damaged());
        }
    }
    VERIFIED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(model.dir.clone());
    Ok(())
}

/// The installed model to hear with: the one named, or the first installed.
pub fn chosen(root: &Path, model_id: Option<&str>) -> Option<Installed> {
    let installed = installed(root);
    match model_id {
        Some(id) => installed.into_iter().find(|model| model.id == id),
        None => installed.into_iter().next(),
    }
}

// ------------------------------------------------------------ the engine

struct Loaded {
    dir: PathBuf,
    recognizer: Arc<Mutex<OfflineRecognizer>>,
    used: Instant,
}

/// The one model in memory. Decoding takes the recognizer's own lock, so a
/// second utterance waits for the first rather than splitting the cores.
static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

fn loaded() -> MutexGuard<'static, Option<Loaded>> {
    LOADED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The recognizer for `model`, loading it (and letting any other go) if it
/// is not the one in memory.
fn recognizer(model: &Installed) -> Result<Arc<Mutex<OfflineRecognizer>>, SpeechError> {
    let dir = model.dir.as_path();
    let mut slot = loaded();
    if let Some(current) = slot.as_mut().filter(|current| current.dir == dir) {
        current.used = Instant::now();
        return Ok(current.recognizer.clone());
    }
    *slot = None;
    let started = Instant::now();
    verify(model)?;
    let unloadable = || {
        SpeechError::Engine("The speech model on the desk could not be loaded. Remove it in Settings and download it again.".into())
    };
    let config = config(model).map_err(|_| unloadable())?;
    let recognizer = OfflineRecognizer::create(&config).ok_or_else(unloadable)?;
    eprintln!(
        "[voice] loaded {}: {}ms",
        dir.display(),
        started.elapsed().as_millis()
    );
    let recognizer = Arc::new(Mutex::new(recognizer));
    *slot = Some(Loaded {
        dir: dir.to_path_buf(),
        recognizer: recognizer.clone(),
        used: Instant::now(),
    });
    let dir = dir.to_path_buf();
    std::thread::spawn(move || let_go_when_idle(&dir));
    Ok(recognizer)
}

/// How the engine runs `model`. A transducer decodes with beam search, the
/// one way it can listen for words, which costs about a tenth more time
/// than greedy decoding; it needs its vocabulary scored, written here.
fn config(model: &Installed) -> std::io::Result<OfflineRecognizerConfig> {
    let dir = model.dir.as_path();
    let path = |file: &str| Some(dir.join(file).to_string_lossy().into_owned());
    let mut config = OfflineRecognizerConfig::default();
    match model.engine {
        Engine::Transducer => {
            config.model_config.transducer = OfflineTransducerModelConfig {
                encoder: path("encoder.int8.onnx"),
                decoder: path("decoder.int8.onnx"),
                joiner: path("joiner.int8.onnx"),
            };
            config.model_config.model_type = Some("nemo_transducer".into());
            let tokens = std::fs::read_to_string(dir.join("tokens.txt"))?;
            std::fs::write(dir.join(VOCABULARY), scored_vocabulary(&tokens))?;
            config.model_config.modeling_unit = Some("bpe".into());
            config.model_config.bpe_vocab = path(VOCABULARY);
            config.decoding_method = Some("modified_beam_search".into());
            config.max_active_paths = 4;
            config.hotwords_score = HOTWORD_SCORE;
        }
        Engine::Whisper => {
            // No language named: Whisper says which it heard.
            config.model_config.whisper = OfflineWhisperModelConfig {
                encoder: path("encoder.int8.onnx"),
                decoder: path("decoder.int8.onnx"),
                task: Some("transcribe".into()),
                ..Default::default()
            };
        }
        Engine::Moonshine => {
            config.model_config.moonshine = OfflineMoonshineModelConfig {
                preprocessor: path("preprocess.onnx"),
                encoder: path("encode.int8.onnx"),
                uncached_decoder: path("uncached_decode.int8.onnx"),
                cached_decoder: path("cached_decode.int8.onnx"),
                ..Default::default()
            };
        }
    }
    config.model_config.tokens = path("tokens.txt");
    // Four threads is where the encoder stops getting faster on a laptop.
    config.model_config.num_threads =
        std::thread::available_parallelism().map_or(2, |cores| cores.get().clamp(1, 4)) as i32;
    Ok(config)
}

/// A transducer's `tokens.txt` (`piece id` per line, in BPE merge order) as
/// the engine's hotword encoder reads a vocabulary: `piece score` per line,
/// splitting each word into the pieces whose scores add up highest. The
/// archives carry no scores, so every piece costs one and an earlier merge a
/// little less: a word becomes the fewest pieces, as BPE mostly makes it and
/// so as the model itself spells it. (Scoring by merge order alone splits a
/// word into many small pieces the model never emits, and Parakeet heard
/// "Parakeek".) A line the encoder could not read would stop the process, so
/// only lines of exactly two fields are kept, and never the blank.
fn scored_vocabulary(tokens: &str) -> String {
    let pieces: Vec<(&str, f64)> = tokens
        .lines()
        .filter_map(
            |line| match line.split_whitespace().collect::<Vec<_>>()[..] {
                [piece, id] if piece != "<blk>" => Some((piece, id.parse().ok()?)),
                _ => None,
            },
        )
        .collect();
    let scale = pieces.len().max(1) as f64 * 64.0;
    pieces
        .iter()
        .map(|(piece, id)| format!("{piece} {:.9}\n", -1.0 - id / scale))
        .collect()
}

/// The words a model listens for, out of `asked` in the order they matter:
/// each kept to letters, digits and the marks inside names, a word asked
/// twice kept once, and no more than [`MAX_HOTWORDS`]. What reaches the
/// engine cannot carry its own syntax (`/` between words, a `:` score, a
/// new line), and a NUL never gets that far.
fn hotwords<'a>(asked: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for word in asked {
        let spoken: String = word
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '\'' | '-' | '.') {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let spoken = spoken
            .split_whitespace()
            .map(|part| part.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let fits = !spoken.is_empty() && spoken.chars().count() <= MAX_HOTWORD_CHARS;
        if fits
            && !kept
                .iter()
                .any(|one| one.to_lowercase() == spoken.to_lowercase())
        {
            kept.push(spoken);
        }
        if kept.len() == MAX_HOTWORDS {
            break;
        }
    }
    kept
}

/// `samples` in pieces of at most `longest`, each cut at the quietest tenth
/// of a second in its last eight seconds, so a cut falls between words.
fn pieces(samples: &[f32], rate: usize, longest: Duration) -> Vec<&[f32]> {
    let longest = (longest.as_secs_f32() * rate as f32) as usize;
    let frame = (rate / 10).max(1);
    let mut pieces = Vec::new();
    let mut rest = samples;
    while rest.len() > longest {
        let window = &rest[..longest];
        let from = longest.saturating_sub(8 * rate);
        let loudness = |start: &usize| -> f32 {
            window[*start..*start + frame]
                .iter()
                .map(|value| value * value)
                .sum()
        };
        let quietest = (from..longest - frame)
            .step_by(frame)
            .min_by(|a, b| loudness(a).total_cmp(&loudness(b)))
            .unwrap_or(from);
        let (piece, after) = rest.split_at((quietest + frame / 2).max(1));
        pieces.push(piece);
        rest = after;
    }
    pieces.push(rest);
    pieces
}

/// Watches the model in `dir` until it is idle for [`IDLE`], or another took its place.
fn let_go_when_idle(dir: &Path) {
    loop {
        std::thread::sleep(IDLE / 5);
        let mut slot = loaded();
        match slot.as_ref() {
            Some(current) if current.dir == dir => {
                if current.used.elapsed() >= IDLE {
                    *slot = None;
                    eprintln!("[voice] let go of {} after it sat idle", dir.display());
                    return;
                }
            }
            _ => return,
        }
    }
}

/// Lets go of the model in `dir`, if it is the one in memory: it is being removed.
pub(crate) fn unload(dir: &Path) {
    let mut slot = loaded();
    if slot.as_ref().is_some_and(|current| current.dir == dir) {
        *slot = None;
    }
    VERIFIED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|verified| verified != dir);
}

/// The words in `samples` (mono, at `rate`), heard by `model` listening for
/// `hotwords` when it is a transducer. Blocking: it runs the model.
fn recognize(
    model: &Installed,
    rate: u32,
    samples: &[f32],
    hotwords: &[String],
) -> Result<String, SpeechError> {
    // Too short for one frame of the encoder, which throws rather than answer.
    let shortest = (rate as usize / 10).max(1);
    if samples.len() < shortest {
        return Ok(String::new());
    }
    let recognizer = recognizer(model)?;
    let recognizer = recognizer.lock().unwrap_or_else(PoisonError::into_inner);
    let pieces = match model.engine.longest_piece() {
        Some(longest) => pieces(samples, rate as usize, longest),
        None => vec![samples],
    };
    let mut heard = Vec::new();
    for piece in pieces.into_iter().filter(|piece| piece.len() >= shortest) {
        let stream = if model.engine == Engine::Transducer && !hotwords.is_empty() {
            recognizer.create_stream_with_hotwords(&hotwords.join("/"))
        } else {
            recognizer.create_stream()
        };
        // The engine resamples anything that is not 16 kHz itself.
        stream.accept_waveform(rate as i32, piece);
        recognizer.decode(&stream);
        let text = stream
            .get_result()
            .map(|result| result.text)
            .ok_or_else(|| {
                SpeechError::Engine(
                    "The speech model on the desk heard nothing it could read.".into(),
                )
            })?;
        heard.push(text.trim().to_string());
    }
    heard.retain(|text| !text.is_empty());
    Ok(heard.join(" "))
}

// ------------------------------------------------------------ the audio

/// Mono samples between -1 and 1, and their rate.
#[derive(Debug, PartialEq)]
struct Samples {
    rate: u32,
    values: Vec<f32>,
}

impl Samples {
    fn from_pcm16(rate: u32, pcm: &[u8]) -> Samples {
        Samples {
            rate,
            values: pcm
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32_768.0)
                .collect(),
        }
    }

    fn too_long(&self) -> bool {
        self.values.len() > MAX_SECONDS * self.rate as usize
    }
}

/// A clip's samples: a mono PCM16 WAV as it is, AAC in MP4 decoded.
fn samples(clip: &Clip) -> Result<Samples, SpeechError> {
    let unsupported = || SpeechError::UnsupportedFormat(clip.mime.clone());
    let samples = match super::base_mime(&clip.mime).as_str() {
        "audio/wav" => {
            let (rate, pcm) = wav::mono_pcm16(&clip.bytes).ok_or_else(unsupported)?;
            Samples::from_pcm16(rate, pcm)
        }
        "audio/mp4" => aac(&clip.bytes).ok_or_else(unsupported)?,
        _ => return Err(unsupported()),
    };
    if samples.too_long() {
        return Err(too_long());
    }
    Ok(samples)
}

fn too_long() -> SpeechError {
    SpeechError::Engine("The desk hears at most a minute at a time.".into())
}

/// The first audio track of an MP4, decoded and mixed down to one channel.
/// `None` when it is not AAC in MP4 or does not decode.
fn aac(bytes: &[u8]) -> Option<Samples> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;
    let track = format.default_track(TrackType::Audio)?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(
            track.codec_params.as_ref()?.audio()?,
            &AudioDecoderOptions::default(),
        )
        .ok()?;
    let mut rate = 0;
    let mut values = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(_) => return None,
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // One damaged packet costs its own few milliseconds, not the clip.
            Err(Error::DecodeError(_)) => continue,
            Err(_) => return None,
        };
        rate = decoded.spec().rate();
        let channels = decoded.spec().channels().count().max(1);
        decoded.copy_to_vec_interleaved(&mut interleaved);
        values.extend(
            interleaved
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        if values.len() > (MAX_SECONDS + 1) * rate as usize {
            break;
        }
    }
    (rate > 0).then_some(Samples { rate, values })
}

// ------------------------------------------------------------ the adapter

/// One installed model, as a listener. It never speaks.
pub struct Local {
    model: Installed,
    clock: TurnClock,
    hotwords: Vec<String>,
}

impl Local {
    pub fn new(model: Installed) -> Local {
        Local {
            model,
            clock: TurnClock::default(),
            hotwords: Vec::new(),
        }
    }

    /// Listening for `asked` too, as [`hotwords`] keeps them. Only a
    /// transducer can; any other model hears as it would without.
    pub fn listening_for(mut self, asked: &[String]) -> Local {
        self.hotwords = hotwords(asked.iter().map(String::as_str));
        self
    }

    pub(crate) fn with_clock(mut self, clock: &TurnClock) -> Local {
        self.clock = clock.clone();
        self
    }

    async fn hear(&self, samples: Samples) -> Result<String, SpeechError> {
        self.clock.heard();
        let model = self.model.clone();
        let hotwords = self.hotwords.clone();
        let started = Instant::now();
        let seconds = samples.values.len() as f32 / samples.rate.max(1) as f32;
        let words = tokio::task::spawn_blocking(move || {
            recognize(&model, samples.rate, &samples.values, &hotwords)
        })
        .await
        .unwrap_or_else(|_| {
            Err(SpeechError::Engine(
                "The speech model on the desk stopped.".into(),
            ))
        });
        eprintln!(
            "[voice] transcribe {}: {:.1}s of audio in {}ms",
            self.id(),
            seconds,
            started.elapsed().as_millis()
        );
        self.clock.unanswered(&words);
        words
    }
}

#[async_trait]
impl Speech for Local {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: PROVIDER_ID.into(),
            model_id: self.model.id.clone(),
            voice: None,
        }
    }

    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError> {
        let samples = samples(&clip)?;
        self.hear(samples).await
    }

    fn supports_live_input(&self) -> bool {
        true
    }

    /// The model hears a whole utterance, so live audio is gathered until
    /// the caller closes it; the call already caps how much that can be.
    async fn transcribe_live(
        &self,
        mut input: tokio::sync::mpsc::Receiver<Vec<u8>>,
        sample_rate: u32,
    ) -> Result<String, SpeechError> {
        let mut pcm = Vec::new();
        while let Some(frame) = input.recv().await {
            pcm.extend_from_slice(&frame);
            if pcm.len() > MAX_SECONDS * RATE * 2 {
                return Err(too_long());
            }
        }
        self.hear(Samples::from_pcm16(sample_rate, &pcm)).await
    }

    async fn speak(&self, _text: &str) -> Result<Clip, SpeechError> {
        Err(SpeechError::WrongJob)
    }
}

/// A model as `installed` would list it, for tests that never run it.
#[cfg(test)]
pub(crate) fn listed(id: &str, name: &str) -> Installed {
    Installed {
        id: id.into(),
        name: name.into(),
        engine: Engine::Transducer,
        dir: PathBuf::from(id),
        files: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
