//! The desk's voice with a stand-in for Phonon: how it is offered, installed
//! and refused, loaded once, kept by a call and let go, streamed and
//! cancelled. Phonon itself speaks only in the ignored measurement at the
//! end, which needs its files.

use super::*;
use crate::speech::local::{IDLE, Installs, catalogue, installed, models_dir};
use axum::Router;
use hotline_contract::SpeechModelState;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// A frame of Phonon's audio: 80 ms at 24 kHz.
const FRAME: usize = 1_920;

/// Says `frames` frames of a quiet tone for any text, counting each it
/// makes, and pausing `pace` before each.
struct StandIn {
    frames: usize,
    pace: Duration,
    made: Arc<AtomicUsize>,
}

impl Speaker for StandIn {
    fn say(
        &self,
        _text: &str,
        _voice: &str,
        piece: &mut dyn FnMut(Vec<f32>) -> bool,
    ) -> Result<(), SpeechError> {
        for _ in 0..self.frames {
            std::thread::sleep(self.pace);
            self.made.fetch_add(1, Ordering::SeqCst);
            if !piece(vec![0.25; FRAME]) {
                break;
            }
        }
        Ok(())
    }
}

fn model(dir: &Path) -> Installed {
    Installed {
        id: MODEL_ID.into(),
        name: "Phonon".into(),
        engine: super::super::Engine::Phonon,
        dir: dir.to_path_buf(),
        files: Vec::new(),
    }
}

/// A voice on a slot of its own, loading `load`.
fn on_slot(slot: &'static Slot<Engine>, load: Load, dir: &Path) -> LocalVoice {
    LocalVoice {
        model: model(dir),
        voice: VOICES[0].name,
        clock: TurnClock::default(),
        slot,
        load,
    }
}

/// Every chunk `speak_chunks` sends, and what it answered.
async fn chunks(voice: &LocalVoice, text: &str) -> (Vec<SpeechChunk>, Result<(), SpeechError>) {
    let (output, mut sent) = mpsc::channel(4);
    let (result, chunks) = tokio::join!(voice.speak_chunks(text, output), async {
        let mut chunks = Vec::new();
        while let Some(chunk) = sent.recv().await {
            chunks.push(chunk);
        }
        chunks
    });
    (chunks, result)
}

/// Samples in a clip the stand-in made.
fn samples(clip: &Clip) -> usize {
    let (rate, pcm) = wav::mono_pcm16(&clip.bytes).expect("a mono PCM16 WAV");
    assert_eq!(rate, RATE);
    pcm.len() / 2
}

#[test]
fn only_phonon_31m_english_is_offered_every_file_pinned_at_one_commit() {
    let offered: Vec<Model> = catalogue()
        .into_iter()
        .filter(|model| model.engine.speaks())
        .collect();
    assert_eq!(offered.len(), usize::from(BUILT));
    let model = offered.first().cloned().unwrap_or_else(super::offered);
    assert_eq!(model.id, "phonon-31m-en");
    let Source::Files(files) = &model.source else {
        panic!("the voice is files, not an archive");
    };
    let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, FILES, "every file the engine reads, and no other");
    assert_eq!(
        model.download_bytes,
        files.iter().map(|file| file.bytes).sum::<u64>()
    );
    for file in files {
        // The two repos whose weights and voices are Apache 2.0, each at one commit:
        // never phonon-40m, which is CC BY-NC, and never a branch that can move.
        assert!(
            file.url.starts_with(MODEL_FILES) || file.url.starts_with(VOICE_FILES),
            "{}",
            file.url
        );
        assert!(!file.url.contains("40m") && !file.url.contains("/main/"));
        assert_eq!(file.sha256.len(), 64);
        assert!(
            file.sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(!file.name.contains(['/', '\\']) && file.bytes > 0);
    }
    assert!(model.credit.contains("Apache 2.0") && model.credit.contains("Kyutai"));
    assert_eq!(voice_named(Some("Edwin")), "Edwin");
    assert_eq!(
        voice_named(Some("eve")),
        VOICES[0].name,
        "a stranger speaks in the first"
    );
    assert_eq!(voice_named(None), VOICES[0].name);
}

/// Serves each of `files` at its name, and `tampered` bytes in place of the one named so.
async fn serve(files: &[(String, Vec<u8>)], tampered: Option<&str>) -> String {
    let mut app = Router::new();
    for (name, body) in files {
        let body = if tampered == Some(name.as_str()) {
            body.iter().map(|byte| byte ^ 1).collect()
        } else {
            body.clone()
        };
        app = app.route(
            &format!("/{name}"),
            axum::routing::get(move || {
                let body = body.clone();
                async move { body }
            }),
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// The voice as `offered` makes it, from `url`, pinned to `files`.
fn offered_from(url: &str, files: &[(String, Vec<u8>)]) -> Model {
    let pinned: Vec<Pinned> = files
        .iter()
        .map(|(name, body)| Pinned {
            name: name.clone(),
            url: format!("{url}/{name}"),
            bytes: body.len() as u64,
            sha256: hex::encode(Sha256::digest(body)),
        })
        .collect();
    Model {
        source: Source::Files(pinned),
        download_bytes: files.iter().map(|(_, body)| body.len() as u64).sum(),
        ..super::offered()
    }
}

/// What `status` says of the voice once its download has settled.
async fn settled(installs: &Installs) -> hotline_contract::SpeechModel {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let model = installs
                .status()
                .into_iter()
                .find(|model| model.id == MODEL_ID)
                .expect("offered");
            if !matches!(
                model.state,
                SpeechModelState::Downloading | SpeechModelState::Unpacking
            ) {
                return model;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the download settles")
}

#[tokio::test]
async fn the_voice_installs_file_by_file_and_a_file_that_is_not_the_pinned_one_is_refused() {
    let files: Vec<(String, Vec<u8>)> = FILES
        .iter()
        .map(|name| {
            (
                name.to_string(),
                format!("the bytes of {name}").into_bytes(),
            )
        })
        .collect();

    // One voice file differs from its pin: nothing is installed, and nothing is left behind.
    let root = tempfile::tempdir().unwrap();
    let url = serve(&files, Some("pippa.safetensors")).await;
    let installs = Installs::new(root.path(), vec![offered_from(&url, &files)]);
    installs.install(MODEL_ID).unwrap();
    let refused = settled(&installs).await;
    assert_eq!(refused.state, SpeechModelState::Available);
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|error| error.contains("not the model Hotline expects")),
        "{refused:?}"
    );
    assert_eq!(installed(root.path()), []);
    let left: Vec<PathBuf> = std::fs::read_dir(models_dir(root.path()))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(left.is_empty(), "{left:?}");

    // Every file as pinned: the voice is installed, its manifest records each file.
    let url = serve(&files, None).await;
    let installs = Installs::new(root.path(), vec![offered_from(&url, &files)]);
    installs.install(MODEL_ID).unwrap();
    let model = settled(&installs).await;
    assert_eq!(model.state, SpeechModelState::Installed, "{model:?}");
    assert_eq!(model.error, None);
    let on_disk = installed(root.path());
    assert_eq!(on_disk.len(), 1);
    assert!(on_disk[0].engine.speaks());
    assert_eq!(on_disk[0].files.len(), FILES.len());
    for (name, body) in &files {
        assert_eq!(&std::fs::read(on_disk[0].dir.join(name)).unwrap(), body);
    }
    // It speaks, so it is never chosen to hear with.
    assert_eq!(super::super::chosen(root.path(), None), None);
    assert_eq!(super::super::chosen(root.path(), Some(MODEL_ID)), None);

    // A file damaged after it was installed is refused before Phonon reads it.
    std::fs::write(on_disk[0].dir.join(WEIGHTS), b"THE BYTES OF model.q8.gguf").unwrap();
    let damaged = super::load(&on_disk[0]).map(drop);
    assert!(
        matches!(&damaged, Err(SpeechError::Engine(sentence)) if sentence.contains("damaged")),
        "{damaged:?}"
    );
}

#[tokio::test]
async fn audio_is_handed_on_a_fifth_of_a_second_first_then_half_a_second_at_a_time() {
    static SLOT: Slot<Engine> = Slot::new();
    let dir = tempfile::tempdir().unwrap();
    // 13 frames: 1.04 s of audio.
    let voice = on_slot(
        &SLOT,
        |_| {
            Ok(Box::new(StandIn {
                frames: 13,
                pace: Duration::ZERO,
                made: Arc::default(),
            }))
        },
        dir.path(),
    );
    let (chunks, result) = chunks(&voice, "Hello there.").await;
    assert_eq!(result, Ok(()));
    let sizes: Vec<usize> = chunks.iter().map(|chunk| samples(&chunk.clip)).collect();
    assert_eq!(sizes, [FIRST_CLIP, CLIP, 13 * FRAME - FIRST_CLIP - CLIP]);
    let finals: Vec<bool> = chunks.iter().map(|chunk| chunk.final_chunk).collect();
    assert_eq!(finals, [false, false, true]);
    assert!(chunks.iter().all(|chunk| chunk.clip.mime == "audio/wav"));

    // Whole, the same audio is one clip.
    let clip = voice.speak("Hello there.").await.unwrap();
    assert_eq!(samples(&clip), 13 * FRAME);
    assert_eq!(voice.speak("  ").await, Err(SpeechError::NothingToSay));
    assert_eq!(voice.transcribe(clip).await, Err(SpeechError::WrongJob));
    assert!(voice.streams_chunks());
    let id = voice.id();
    assert_eq!(
        (
            id.provider_id.as_str(),
            id.model_id.as_str(),
            id.voice.as_deref()
        ),
        (super::super::PROVIDER_ID, MODEL_ID, Some("Hadley"))
    );
}

#[tokio::test]
async fn the_model_is_loaded_once_for_every_sentence_and_a_failed_load_says_why() {
    static SLOT: Slot<Engine> = Slot::new();
    static LOADS: AtomicUsize = AtomicUsize::new(0);
    let dir = tempfile::tempdir().unwrap();
    let voice = on_slot(
        &SLOT,
        |_| {
            LOADS.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(StandIn {
                frames: 2,
                pace: Duration::ZERO,
                made: Arc::default(),
            }))
        },
        dir.path(),
    );
    for _ in 0..3 {
        assert_eq!(chunks(&voice, "Again.").await.1, Ok(()));
    }
    voice.speak("And whole.").await.unwrap();
    assert_eq!(LOADS.load(Ordering::SeqCst), 1);

    static NEVER: Slot<Engine> = Slot::new();
    let broken = on_slot(
        &NEVER,
        |_| Err(SpeechError::Engine("could not be loaded".into())),
        dir.path(),
    );
    assert_eq!(
        chunks(&broken, "Hello.").await.1,
        Err(SpeechError::Engine("could not be loaded".into()))
    );
}

#[tokio::test]
async fn a_call_keeps_the_voice_loaded_while_it_is_open_and_it_is_let_go_once_idle_after() {
    static SLOT: Slot<Engine> = Slot::new();
    static LOADS: AtomicUsize = AtomicUsize::new(0);
    let dir = tempfile::tempdir().unwrap();
    let voice = Arc::new(on_slot(
        &SLOT,
        |_| {
            LOADS.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(StandIn {
                frames: 1,
                pace: Duration::ZERO,
                made: Arc::default(),
            }))
        },
        dir.path(),
    ));
    let until = CancellationToken::new();
    let ready = tokio::spawn({
        let (voice, until) = (voice.clone(), until.clone());
        async move { voice.stay_ready(until).await }
    });
    // The call's start loads it, before anything is said.
    tokio::time::timeout(Duration::from_secs(5), async {
        while SLOT.loaded().is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("loaded as the call starts");
    assert_eq!(LOADS.load(Ordering::SeqCst), 1);

    // A work turn longer than IDLE, with the call open: still loaded.
    let long_ago = || Instant::now().checked_sub(IDLE * 2).unwrap();
    SLOT.loaded().as_mut().unwrap().used = long_ago();
    assert!(SLOT.let_go_if_idle(dir.path()));
    assert!(SLOT.loaded().is_some(), "kept while the call is open");
    assert!(!ready.is_finished());

    // The call ends; once idle, it is let go, and the next sentence loads it again.
    until.cancel();
    ready.await.unwrap();
    SLOT.loaded().as_mut().unwrap().used = long_ago();
    assert!(!SLOT.let_go_if_idle(dir.path()));
    assert!(SLOT.loaded().is_none(), "let go once idle with no call");
    voice.speak("Back.").await.unwrap();
    assert_eq!(LOADS.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn hanging_up_mid_sentence_stops_the_model_within_a_few_frames() {
    static SLOT: Slot<Engine> = Slot::new();
    static MADE: std::sync::OnceLock<Arc<AtomicUsize>> = std::sync::OnceLock::new();
    let made = MADE.get_or_init(Arc::default).clone();
    let dir = tempfile::tempdir().unwrap();
    // Ten seconds of audio, a frame every 5 ms.
    let voice = on_slot(
        &SLOT,
        |_| {
            Ok(Box::new(StandIn {
                frames: 125,
                pace: Duration::from_millis(5),
                made: MADE.get().expect("set before the voice speaks").clone(),
            }))
        },
        dir.path(),
    );
    let (output, mut sent) = mpsc::channel(1);
    let speaking = tokio::spawn(async move { voice.speak_chunks("A long answer.", output).await });
    let first = sent.recv().await.expect("the first clip");
    assert!(!first.final_chunk);
    drop(sent);
    assert_eq!(speaking.await.unwrap(), Err(SpeechError::Cancelled));

    // The generation thread notices at its next frame, after at most the
    // frames already queued for the closed channel.
    let stopped = Arc::new(AtomicBool::new(false));
    tokio::time::timeout(Duration::from_secs(5), {
        let (made, stopped) = (made.clone(), stopped.clone());
        async move {
            let mut last = made.load(Ordering::SeqCst);
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let now = made.load(Ordering::SeqCst);
                if now == last {
                    stopped.store(true, Ordering::SeqCst);
                    return;
                }
                last = now;
            }
        }
    })
    .await
    .expect("the model stops");
    assert!(stopped.load(Ordering::SeqCst));
    let frames = made.load(Ordering::SeqCst);
    assert!(frames < 40, "made {frames} of 125 frames");
    // The engine's lock is free again for the next sentence.
    assert!(SLOT.loaded().as_ref().unwrap().engine.try_lock().is_ok());
}

/// Phonon itself on this machine, through the adapter a call uses: load,
/// then the first clip and the whole of a 6-word and a 30-word sentence,
/// five times each, with the cores kept busy and the memory the process
/// holds. `PHONON_DIR` is a directory with the voice's files as the desk
/// keeps them ([`FILES`]). Run it optimised:
///
/// ```text
/// PHONON_DIR=… cargo test --release -p hotline-voice phonon_on_this_machine -- --ignored --nocapture
/// ```
#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Phonon's files in PHONON_DIR"]
async fn phonon_on_this_machine() {
    static SLOT: Slot<Engine> = Slot::new();
    let dir = PathBuf::from(std::env::var("PHONON_DIR").expect("PHONON_DIR"));
    // The process's resident memory in MB and CPU seconds so far, as ps says.
    let ps = || {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=,time=", "-p", &std::process::id().to_string()])
            .output()
            .expect("ps runs");
        let text = String::from_utf8(out.stdout).unwrap();
        let mut fields = text.split_whitespace();
        let rss_mb = fields.next().unwrap().parse::<f64>().unwrap() / 1024.0;
        let cpu_seconds = fields.next().unwrap().split(':').fold(0.0, |total, part| {
            total * 60.0 + part.parse::<f64>().unwrap()
        });
        (rss_mb, cpu_seconds)
    };
    let voice = on_slot(&SLOT, phonon::load, &dir);
    let (before_mb, _) = ps();
    let started = Instant::now();
    let model = model(&dir);
    tokio::task::spawn_blocking(move || SLOT.get(&model.dir, || phonon::load(&model)).map(drop))
        .await
        .unwrap()
        .expect("Phonon loads");
    let (loaded_mb, _) = ps();
    println!(
        "load {}ms, memory {before_mb:.0} -> {loaded_mb:.0} MB",
        started.elapsed().as_millis()
    );
    let sentences = [
        "Okay, I will check that now.",
        "The build finished on all three platforms, but the Windows leg took twice as long as yesterday, so I opened a task to find out why before the release today.",
    ];
    for text in sentences {
        let words = text.split_whitespace().count();
        for run in 1..=5 {
            let (_, cpu_before) = ps();
            let started = Instant::now();
            let (output, mut sent) = mpsc::channel(4);
            let (result, (first, audio)) = tokio::join!(voice.speak_chunks(text, output), async {
                let (mut first, mut audio) = (None, 0);
                while let Some(chunk) = sent.recv().await {
                    first.get_or_insert(started.elapsed());
                    audio += samples(&chunk.clip);
                }
                (first, audio)
            });
            result.unwrap();
            let total = started.elapsed();
            let (mb, cpu_after) = ps();
            println!(
                "{words} words, run {run}: first clip {}ms, done {}ms, {:.2}s of audio, {:.1} cores busy, memory {mb:.0} MB",
                first.unwrap().as_millis(),
                total.as_millis(),
                audio as f64 / f64::from(RATE),
                (cpu_after - cpu_before) / total.as_secs_f64(),
            );
        }
    }
}
