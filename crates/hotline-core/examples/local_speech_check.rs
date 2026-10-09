//! Downloads one of the desk's own speech models from where Hotline really
//! fetches it, verifies and unpacks it as the desk does, and hears the speech
//! fixtures with it, on a scratch data directory: the check that the pinned
//! archive is still there and still the one pinned, and that the engine links
//! and runs on this machine. The timings are what a person would wait, and
//! the real-time factor is that time over the length of the speech.
//! Any further arguments are mono WAV files to hear after the fixtures.
//!
//! cargo run --release -p hotline-core --example local_speech_check -- parakeet-tdt-110m-en [clip.wav ...]

use base64::{Engine, engine::general_purpose::STANDARD};
use hotline_core::{contract::SpeechModelState, desk::Desk, wire::RoomHandle};
use std::path::Path;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "parakeet-tdt-110m-en".into());
    let clips: Vec<std::path::PathBuf> = std::env::args().skip(2).map(Into::into).collect();
    let root = tempfile::tempdir().expect("a scratch data directory");
    let desk = Desk::open(root.path()).expect("a desk on the scratch directory");
    let voice = desk.voice().expect("the desk's voice");

    let started = Instant::now();
    voice.install_speech_model(&id).expect("an offered model");
    let mut reported = Instant::now();
    loop {
        let model = voice
            .speech_models()
            .into_iter()
            .find(|model| model.id == id)
            .expect("the model is listed");
        if let Some(error) = model.error {
            eprintln!("{id} did not install: {error}");
            std::process::exit(1);
        }
        if model.state == SpeechModelState::Installed {
            break;
        }
        if reported.elapsed() > Duration::from_secs(2) {
            eprintln!(
                "{id}: {:?}, {} of {} MB",
                model.state,
                model.received_bytes.unwrap_or_default() / 1_000_000,
                model.download_bytes / 1_000_000
            );
            reported = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!(
        "{id}: downloaded, verified and unpacked in {:.1}s",
        started.elapsed().as_secs_f32()
    );

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice");
    let heard = [
        ("ask-mack.wav", "audio/wav", 2.39),
        ("ask-mack.wav", "audio/wav", 2.39),
        ("ask-mack.m4a", "audio/mp4", 2.39),
        ("acknowledgement.wav", "audio/wav", 2.79),
    ]
    .into_iter()
    .map(|(file, mime, seconds)| (fixtures.join(file), mime, seconds))
    .chain(clips.into_iter().map(|clip| {
        let seconds = std::fs::metadata(&clip).expect("a clip").len() as f32 / 32_000.0;
        (clip, "audio/wav", seconds)
    }));
    for (path, mime, seconds) in heard {
        let data = STANDARD.encode(std::fs::read(&path).expect("a clip"));
        let started = Instant::now();
        let words = voice.transcribe(mime, &data).await;
        let elapsed = started.elapsed();
        println!(
            "{}: {words:?} in {}ms, real-time factor {:.3}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            elapsed.as_millis(),
            elapsed.as_secs_f32() / seconds
        );
    }
}
