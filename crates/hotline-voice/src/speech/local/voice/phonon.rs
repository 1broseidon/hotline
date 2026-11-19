//! Phonon itself, through `ptts`: built only where `ptts` builds
//! ([`super::BUILT`]).

use super::{Engine, FILES, Installed, Speaker, SpeechError, VOICES, WEIGHTS, voice_file};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Quant, SpeechOptions, Synth};
use std::time::Instant;

/// Threads the model's arithmetic runs on, the caller's among them. Phonon
/// spoke twice as fast on four as on one on an M-series Mac, and slower on
/// every core than on four: xn's pool hands each small kernel to all its
/// threads, which costs more than it saves past a few.
const THREADS: usize = 4;

struct Phonon(Synth);

impl Speaker for Phonon {
    fn say(
        &self,
        text: &str,
        voice: &str,
        piece: &mut dyn FnMut(Vec<f32>) -> bool,
    ) -> Result<(), SpeechError> {
        let failed = |_| SpeechError::Engine("The desk's own voice could not say that.".into());
        let options = SpeechOptions::default().voice(voice);
        // Dropping the stream stops the model and waits for its threads.
        for audio in self.0.stream_with(text, &options).map_err(failed)? {
            if !piece(audio.map_err(failed)?) {
                break;
            }
        }
        Ok(())
    }
}

/// Phonon on the CPU with the eight-bit weights and every voice offered,
/// reading English: numbers, amounts and dates are said as words.
pub(super) fn load(model: &Installed) -> Result<Engine, SpeechError> {
    let started = Instant::now();
    start_threads();
    let unloadable = |_| {
        SpeechError::Engine(
            "The desk's own voice could not be loaded. Remove it in Settings and download it again."
                .into(),
        )
    };
    let dir = &model.dir;
    let config = ptts::checkpoint::read_config(dir.join(FILES[0])).map_err(unloadable)?;
    let mut builder = Synth::builder(config, dir.join(WEIGHTS), Lang::En)
        .tokenizer_file(dir.join(FILES[1]))
        .device(DeviceKind::Cpu)
        .quant(Quant::Q80);
    for voice in &VOICES {
        builder = builder.add_voice(voice.name, dir.join(voice_file(voice)));
    }
    let synth = builder.build().map_err(unloadable)?;
    eprintln!(
        "[voice] loaded {}: {}ms",
        dir.display(),
        started.elapsed().as_millis()
    );
    Ok(Box::new(Phonon(synth)))
}

/// xn's pool of [`THREADS`], started before the model's first arithmetic.
/// Left to itself the pool takes every core, sized from an environment
/// variable it would set for the whole process. Its threads live as long as
/// the process and sleep when there is nothing to say; a second call is a
/// no-op.
fn start_threads() {
    let config = xn::threadpool::PoolConfig {
        workers: THREADS - 1,
        spin_budget: None,
    };
    xn::threadpool::start_pool_with(config, |work| {
        std::thread::Builder::new()
            .name("phonon".into())
            .spawn(work)
            .expect("the desk can start a thread for its voice");
    });
}
