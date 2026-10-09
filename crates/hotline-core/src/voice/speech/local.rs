//! The desk's own ears: speech to text on the desk's machine, with a model
//! the owner downloaded (`install.rs`). Nothing said leaves the machine and
//! nothing is charged, so it is the provider `local` at a price of zero.
//!
//! The engine is sherpa-onnx with NVIDIA's Parakeet transducers, linked
//! statically with onnxruntime, so it runs wherever the desk does with nothing
//! installed beside it. A model on disk describes itself (`model.json`), so
//! hearing never needs the download catalogue: what is in the directory is
//! what can hear.
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
//! second is heard as nothing without running the model.

mod install;

pub use install::{Installs, Model, catalogue};

use super::{Clip, Speech, SpeechError, SpeechId, TurnClock, wav};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub const PROVIDER_ID: &str = "local";
/// What the picker calls it. The desk's machine, said as the window says it:
/// a phone or a window on another computer hears through it too.
pub const PROVIDER_NAME: &str = "On the desk";

/// The files of a model, the same for every one: a NeMo transducer as
/// sherpa-onnx exports it, quantized to eight bits.
const FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];
const MANIFEST: &str = "model.json";

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
    let whole = FILES.iter().all(|file| {
        let recorded = manifest.files.iter().find(|one| one.name == *file);
        let size = std::fs::metadata(dir.join(file))
            .ok()
            .filter(|meta| meta.is_file());
        matches!((recorded, size), (Some(recorded), Some(size)) if recorded.bytes == size.len())
    });
    (named && whole).then(|| Installed {
        id: manifest.id,
        name: manifest.name,
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
    let path = |file: &str| Some(dir.join(file).to_string_lossy().into_owned());
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: path(FILES[0]),
        decoder: path(FILES[1]),
        joiner: path(FILES[2]),
    };
    config.model_config.tokens = path(FILES[3]);
    config.model_config.model_type = Some("nemo_transducer".into());
    // Four threads is where the encoder stops getting faster on a laptop.
    config.model_config.num_threads =
        std::thread::available_parallelism().map_or(2, |cores| cores.get().clamp(1, 4)) as i32;
    let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
        SpeechError::Engine("The speech model on the desk could not be loaded. Remove it in Settings and download it again.".into())
    })?;
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

/// The words in `samples` (mono, at `rate`), heard by `model`. Blocking: it
/// runs the model.
fn recognize(model: &Installed, rate: u32, samples: &[f32]) -> Result<String, SpeechError> {
    // Too short for one frame of the encoder, which throws rather than answer.
    if samples.len() < (rate as usize / 10).max(1) {
        return Ok(String::new());
    }
    let recognizer = recognizer(model)?;
    let recognizer = recognizer.lock().unwrap_or_else(PoisonError::into_inner);
    let stream = recognizer.create_stream();
    // The engine resamples anything that is not 16 kHz itself.
    stream.accept_waveform(rate as i32, samples);
    recognizer.decode(&stream);
    let text = stream.get_result().map(|result| result.text);
    text.map(|text| text.trim().to_string()).ok_or_else(|| {
        SpeechError::Engine("The speech model on the desk heard nothing it could read.".into())
    })
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
}

impl Local {
    pub fn new(model: Installed) -> Local {
        Local {
            model,
            clock: TurnClock::default(),
        }
    }

    pub(crate) fn with_clock(mut self, clock: &TurnClock) -> Local {
        self.clock = clock.clone();
        self
    }

    async fn hear(&self, samples: Samples) -> Result<String, SpeechError> {
        self.clock.heard();
        let model = self.model.clone();
        let started = Instant::now();
        let seconds = samples.values.len() as f32 / samples.rate.max(1) as f32;
        let words =
            tokio::task::spawn_blocking(move || recognize(&model, samples.rate, &samples.values))
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
        dir: PathBuf::from(id),
        files: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
