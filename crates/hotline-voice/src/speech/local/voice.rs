//! The desk's own voice: text to speech on the desk's machine with Gradium's
//! Phonon 31M English, run in this process by Gradium's `ptts` crate. It is
//! the provider `local` speaking, at a price of zero, and nothing said leaves
//! the machine.
//!
//! The model and the voices it offers are downloaded like a model that
//! hears ([`offered`], through `install.rs`), each file pinned by its size
//! and SHA-256 at one commit of Gradium's two Hugging Face repositories, and
//! hashed again once per run before it loads. Only the 31M English model is
//! offered: its weights, tokenizer and voices are Apache 2.0, where Phonon's
//! 40M models are not licensed for commercial use.
//!
//! The model is loaded when a call that speaks with it starts
//! ([`Speech::stay_ready`]), kept while that call is open and let go after
//! sitting idle, in a slot of its own beside the one a model that hears is
//! kept in, so a call can keep both. Audio is handed on as it is made, a
//! fifth of a second first and then half a second at a time, as xAI's voice
//! streams it; one sentence of a few words takes a few dozen milliseconds on
//! a laptop, so the first clip is ready well before a network voice's.

#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
mod phonon;

use super::{Installed, Slot, install::Model, install::Pinned, install::Source};
use crate::speech::{Clip, Speech, SpeechChunk, SpeechError, SpeechId, TurnClock, wav};
use async_trait::async_trait;
use std::sync::PoisonError;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Whether this build can speak with Phonon. Its matrix kernels (xn-gemm
/// 0.20) include half-precision assembly that needs the FP16 extension,
/// which aarch64 Linux's baseline does not promise, so `ptts` is not built
/// there and the voice is not offered.
pub(super) const BUILT: bool = cfg!(not(all(target_os = "linux", target_arch = "aarch64")));

pub const MODEL_ID: &str = "phonon-31m-en";

/// What Phonon makes: mono samples at 24 kHz.
const RATE: u32 = 24_000;
/// The first clip goes out after a fifth of a second of audio, and every
/// later one after half a second, as xAI's streamed voice does.
const FIRST_CLIP: usize = RATE as usize / 5;
const CLIP: usize = RATE as usize / 2;

/// One voice Phonon speaks in: a name the picker shows, and the file in
/// gradium/phonon-voices that holds it.
pub struct Voice {
    pub name: &'static str,
    /// Its id in gradium/phonon-voices, which names its file there.
    id: &'static str,
    sha256: &'static str,
}

/// The voices offered, the first spoken in when none is chosen: neutral
/// adult voices of Gradium's 25 English ones, two American women, a British
/// woman, two American men and a British man, their median pitch from
/// about 230 Hz (Pippa) down to about 105 Hz (Preston). Each is 256,136
/// bytes.
pub const VOICES: [Voice; 6] = [
    Voice {
        name: "Hadley",
        id: "bXwrkHhURL1VFJdG",
        sha256: "a879938aa9a2df0a8220bdadef88137929f71e887bd91e7db205bd5fa11661d2",
    },
    Voice {
        name: "Delia",
        id: "nGzMHMPAWGaJFHOo",
        sha256: "a0c3dcb91b2247eca56536be9ae8e8becc1c807b1136c5fe101552d6a155b24a",
    },
    Voice {
        name: "Pippa",
        id: "uem82D50GRv2Dwma",
        sha256: "d41a8f01d6e33450f1ce062d332914da10beee99e585dab531750706f187c48a",
    },
    Voice {
        name: "Preston",
        id: "qIxlZ7Ei8KvWlyQH",
        sha256: "6e04b84d3ae8751b22bca0a4f63936f9cb648f4f2182b2d0788071ea1c0b68a1",
    },
    Voice {
        name: "Trevor",
        id: "0zLApsZfVLLNaEGl",
        sha256: "dc1341c8937af5534e6bbcc5c0fc5e7c10312229b59061228074f93683fe2261",
    },
    Voice {
        name: "Edwin",
        id: "zUCxzrOrugTSis03",
        sha256: "ff1d0735b8b45710462b1d1f846da040f970c6eb62c7c2f36dbfb8e5bde74f1e",
    },
];
const VOICE_BYTES: u64 = 256_136;

/// The model's files as the desk keeps them: Gradium's config, tokenizer and
/// eight-bit weights, then one file per voice, named for the voice. The
/// eight-bit weights load in a tenth of the time of the full ones and speak
/// a third faster, with no difference measured in length, loudness or
/// silence on the same sentences.
pub(super) const FILES: [&str; 9] = [
    "config.json",
    "tokenizer.json",
    "model.q8.gguf",
    "hadley.safetensors",
    "delia.safetensors",
    "pippa.safetensors",
    "preston.safetensors",
    "trevor.safetensors",
    "edwin.safetensors",
];
const WEIGHTS: &str = "model.q8.gguf";

/// gradium/phonon-31m-en at the commit of 2026-11-17, whose files are the
/// ones ptts 0.7.1 pins as its default model (92741c5b) plus the licence.
const MODEL_FILES: &str =
    "https://huggingface.co/gradium/phonon-31m-en/resolve/8f7a37c94a4c58b04c8287300c53f11d2e208eae";
/// gradium/phonon-voices at the commit of the same week.
const VOICE_FILES: &str =
    "https://huggingface.co/gradium/phonon-voices/resolve/0ee17205f254b5abc43b266c8a0f215565c352f7";

/// The file a voice is kept in.
fn voice_file(voice: &Voice) -> String {
    format!("{}.safetensors", voice.name.to_ascii_lowercase())
}

/// The voice as the desk offers it for download: every file pinned.
pub(super) fn offered() -> Model {
    let pinned = |name: &str, url: String, bytes: u64, sha256: &str| Pinned {
        name: name.into(),
        url,
        bytes,
        sha256: sha256.into(),
    };
    let mut files = vec![
        pinned(
            "config.json",
            format!("{MODEL_FILES}/config.json"),
            4_634,
            "b5d0f88353638370d94035b7d73d7d6577c0554d78f8d7ecfd40490aae39ae85",
        ),
        pinned(
            "tokenizer.json",
            format!("{MODEL_FILES}/tokenizer.json"),
            277_967,
            "691a00068fa7b989f2f3e5ae5daa2d1a5bb319f2d5360deafcfb19a8bb6e7936",
        ),
        pinned(
            WEIGHTS,
            format!("{MODEL_FILES}/{WEIGHTS}"),
            64_512_352,
            "a7d9aad2cd6dc66473f24a08bc6d2939030ef4d751ef82349361fbef3272dbae",
        ),
    ];
    for voice in &VOICES {
        files.push(pinned(
            &voice_file(voice),
            format!("{VOICE_FILES}/voices/{}.safetensors", voice.id),
            VOICE_BYTES,
            voice.sha256,
        ));
    }
    let bytes = files.iter().map(|file| file.bytes).sum();
    Model {
        id: MODEL_ID.into(),
        name: "Phonon".into(),
        tag: Some("voice".into()),
        detail: "Speaks English".into(),
        engine: super::Engine::Phonon,
        source: Source::Files(files),
        download_bytes: bytes,
        disk_bytes: bytes,
        credit: "Gradium Phonon 31M English and its voices, Apache 2.0, built on Kyutai's Pocket TTS and Mimi".into(),
        licence_url: "https://www.apache.org/licenses/LICENSE-2.0".into(),
        more_languages: None,
    }
}

/// The voice called `name`, or the first offered when it is none of them.
pub(crate) fn voice_named(name: Option<&str>) -> &'static str {
    VOICES
        .iter()
        .find(|voice| Some(voice.name) == name)
        .unwrap_or(&VOICES[0])
        .name
}

// ------------------------------------------------------------ the engine

/// What speaks: Phonon, or a test's stand-in for it.
pub(super) trait Speaker: Send {
    /// Says `text` in `voice`, handing each piece of audio (mono, at
    /// [`RATE`]) to `piece` as it is made, and stopping as soon as `piece`
    /// answers false. Blocking: it runs the model.
    fn say(
        &self,
        text: &str,
        voice: &str,
        piece: &mut dyn FnMut(Vec<f32>) -> bool,
    ) -> Result<(), SpeechError>;
}

type Engine = Box<dyn Speaker>;
type Load = fn(&Installed) -> Result<Engine, SpeechError>;

/// The one voice in memory. Speaking takes the engine's lock, so a second
/// sentence waits for the first rather than splitting the cores.
pub(super) static SPEAKING: Slot<Engine> = Slot::new();

/// Phonon for `model`, its files checked first.
fn load(model: &Installed) -> Result<Engine, SpeechError> {
    super::verify(model)?;
    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    return phonon::load(model);
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    Err(SpeechError::Engine(
        "The desk's own voice does not run on this computer.".into(),
    ))
}

// ------------------------------------------------------------ the adapter

/// The installed voice model, speaking in one of its voices. It never hears.
pub struct LocalVoice {
    model: Installed,
    voice: &'static str,
    clock: TurnClock,
    slot: &'static Slot<Engine>,
    load: Load,
}

impl LocalVoice {
    /// Speaks with `model` in the voice called `voice`, or the first offered.
    pub fn new(model: Installed, voice: Option<&str>) -> LocalVoice {
        LocalVoice {
            model,
            voice: voice_named(voice),
            clock: TurnClock::default(),
            slot: &SPEAKING,
            load,
        }
    }

    pub(crate) fn with_clock(mut self, clock: &TurnClock) -> LocalVoice {
        self.clock = clock.clone();
        self
    }

    /// Says `text` on a blocking thread, the model loaded first if it is not
    /// in memory, sending each piece of audio to `audio` as it is made. The
    /// model stops within a frame of `audio` being dropped.
    fn generate(
        &self,
        text: &str,
        audio: mpsc::Sender<Vec<f32>>,
    ) -> tokio::task::JoinHandle<Result<(), SpeechError>> {
        let (model, voice, slot, load) = (self.model.clone(), self.voice, self.slot, self.load);
        let text = text.to_string();
        tokio::task::spawn_blocking(move || {
            let engine = slot.get(&model.dir, || load(&model))?;
            let engine = engine.lock().unwrap_or_else(PoisonError::into_inner);
            engine.say(&text, voice, &mut |piece| {
                audio.blocking_send(piece).is_ok()
            })
        })
    }

    /// The whole of `text`, said, as clips of [`FIRST_CLIP`] and then
    /// [`CLIP`] samples sent to `output` as they fill, and the rest last.
    async fn stream(
        &self,
        text: &str,
        output: &mpsc::Sender<SpeechChunk>,
    ) -> Result<(), SpeechError> {
        let started = Instant::now();
        let (audio, mut made) = mpsc::channel(8);
        let generating = self.generate(text, audio);
        let mut pending: Vec<f32> = Vec::new();
        let mut first = None;
        while let Some(piece) = made.recv().await {
            pending.extend(piece);
            loop {
                let size = if first.is_none() { FIRST_CLIP } else { CLIP };
                // Strictly more, so something is always left for the final clip.
                if pending.len() <= size {
                    break;
                }
                let clip: Vec<f32> = pending.drain(..size).collect();
                if first.is_none() {
                    first = Some(started.elapsed());
                    self.clock.first_clip(&self.id());
                }
                let chunk = SpeechChunk {
                    clip: wav_clip(&clip),
                    final_chunk: false,
                };
                output
                    .send(chunk)
                    .await
                    .map_err(|_| SpeechError::Cancelled)?;
            }
        }
        finished(generating.await)?;
        if pending.is_empty() {
            return Err(said_nothing());
        }
        if first.is_none() {
            first = Some(started.elapsed());
            self.clock.first_clip(&self.id());
        }
        let chunk = SpeechChunk {
            clip: wav_clip(&pending),
            final_chunk: true,
        };
        output
            .send(chunk)
            .await
            .map_err(|_| SpeechError::Cancelled)?;
        eprintln!(
            "[voice] speak stream {}: first audio {}ms, done {}ms",
            self.id(),
            first.unwrap_or_default().as_millis(),
            started.elapsed().as_millis()
        );
        Ok(())
    }
}

/// What a generation thread came to: its own answer, or that it stopped.
fn finished(
    joined: Result<Result<(), SpeechError>, tokio::task::JoinError>,
) -> Result<(), SpeechError> {
    joined.unwrap_or_else(|_| Err(SpeechError::Engine("The desk's own voice stopped.".into())))
}

fn said_nothing() -> SpeechError {
    SpeechError::Engine("The desk's own voice said nothing.".into())
}

/// Samples between -1 and 1 as a playable 16-bit WAV.
fn wav_clip(samples: &[f32]) -> Clip {
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|sample| {
            let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "clamped to i16's range above"
            )]
            let value = scaled as i16;
            value.to_le_bytes()
        })
        .collect();
    Clip {
        mime: "audio/wav".into(),
        bytes: wav::pcm16_wav(&pcm, RATE),
    }
}

#[async_trait]
impl Speech for LocalVoice {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: super::PROVIDER_ID.into(),
            model_id: self.model.id.clone(),
            voice: Some(self.voice.into()),
        }
    }

    async fn transcribe(&self, _clip: Clip) -> Result<String, SpeechError> {
        Err(SpeechError::WrongJob)
    }

    /// Loads the model, saying nothing, and keeps it until `until`. A
    /// sentence that arrives during the load waits for it ([`Slot::get`]).
    async fn stay_ready(&self, until: CancellationToken) {
        let _kept = self.slot.keep(&self.model.dir);
        let (model, slot, load) = (self.model.clone(), self.slot, self.load);
        let loading =
            tokio::task::spawn_blocking(move || slot.get(&model.dir, || load(&model)).map(drop));
        tokio::select! {
            () = until.cancelled() => return,
            loaded = loading => {
                if let Ok(Err(error)) = loaded {
                    eprintln!("[voice] could not load {} ahead: {error}", self.model.dir.display());
                }
            }
        }
        until.cancelled().await;
    }

    fn streams_chunks(&self) -> bool {
        true
    }

    async fn speak(&self, text: &str) -> Result<Clip, SpeechError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(SpeechError::NothingToSay);
        }
        let (audio, mut made) = mpsc::channel(8);
        let generating = self.generate(text, audio);
        let mut samples = Vec::new();
        while let Some(piece) = made.recv().await {
            samples.extend(piece);
        }
        finished(generating.await)?;
        if samples.is_empty() {
            return Err(said_nothing());
        }
        self.clock.first_clip(&self.id());
        Ok(wav_clip(&samples))
    }

    /// Dropping `output`'s receiver, or this future, drops the audio the
    /// model is handing on, and the model stops within a frame (80 ms).
    async fn speak_chunks(
        &self,
        text: &str,
        output: mpsc::Sender<SpeechChunk>,
    ) -> Result<(), SpeechError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(SpeechError::NothingToSay);
        }
        if output.is_closed() {
            return Err(SpeechError::Cancelled);
        }
        tokio::select! {
            () = output.closed() => Err(SpeechError::Cancelled),
            result = self.stream(text, &output) => result,
        }
    }
}

#[cfg(test)]
mod tests;
