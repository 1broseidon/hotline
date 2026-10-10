//! What counts as an installed model, and how a clip becomes samples. The
//! model itself runs in the harness (`tests/local_speech.rs`), where a real
//! one can be given.

use super::*;
use crate::voice::speech::wav::pcm16_wav;
use sha2::{Digest, Sha256};

/// A transducer's files, which most tests here place.
const FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// A model's directory as unpacking leaves it, with `files` present. They are
/// not a model the engine could load, so no test here lets it try.
fn place(root: &Path, id: &str, name: &str, files: &[&str]) -> PathBuf {
    place_as(root, id, name, Engine::Transducer, files)
}

fn place_as(root: &Path, id: &str, name: &str, engine: Engine, files: &[&str]) -> PathBuf {
    let dir = models_dir(root).join(id);
    std::fs::create_dir_all(&dir).unwrap();
    for file in files {
        std::fs::write(dir.join(file), b"model").unwrap();
    }
    let manifest = Manifest {
        id: id.into(),
        name: name.into(),
        engine,
        files: engine
            .files()
            .iter()
            .map(|file| FileHash {
                name: file.to_string(),
                bytes: 5,
                sha256: hex::encode(Sha256::digest(b"model")),
            })
            .collect(),
    };
    std::fs::write(dir.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();
    dir
}

fn listener(root: &Path, id: &str) -> Local {
    Local::new(chosen(root, Some(id)).expect("installed"))
}

#[test]
fn a_model_is_installed_only_whole_and_under_its_own_name() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        installed(root.path()).is_empty(),
        "no directory is no model"
    );

    place(
        root.path(),
        "parakeet-tdt-110m-en",
        "Parakeet English",
        &FILES,
    );
    // A download cut short, and a directory that claims another model's name.
    place(root.path(), "half", "Half", &FILES[..3]);
    let impostor = place(root.path(), "impostor", "Impostor", &FILES);
    let claim = std::fs::read_to_string(impostor.join(MANIFEST))
        .unwrap()
        .replace("\"impostor\"", "\"parakeet-tdt-0.6b-v3\"");
    std::fs::write(impostor.join(MANIFEST), claim).unwrap();
    // A file cut short is not the file that was unpacked.
    let short = place(root.path(), "short", "Short", &FILES);
    std::fs::write(short.join(FILES[0]), b"mod").unwrap();
    std::fs::create_dir_all(models_dir(root.path()).join("parakeet-tdt-0.6b-v3.unpacking"))
        .unwrap();

    let found = installed(root.path());
    assert_eq!(
        found
            .iter()
            .map(|model| (model.id.as_str(), model.name.as_str()))
            .collect::<Vec<_>>(),
        [("parakeet-tdt-110m-en", "Parakeet English")]
    );
    assert_eq!(chosen(root.path(), Some("half")), None);
    assert_eq!(
        chosen(root.path(), None).map(|model| model.id),
        Some("parakeet-tdt-110m-en".to_string())
    );
}

#[test]
fn the_most_accurate_installed_model_comes_first_and_strangers_last() {
    let root = tempfile::tempdir().unwrap();
    place(root.path(), "a-retired-model", "Old", &FILES);
    place(
        root.path(),
        "parakeet-tdt-110m-en",
        "Parakeet English",
        &FILES,
    );
    place(root.path(), "parakeet-tdt-0.6b-v3", "Parakeet", &FILES);
    let ids: Vec<String> = installed(root.path())
        .into_iter()
        .map(|model| model.id)
        .collect();
    assert_eq!(
        ids,
        [
            "parakeet-tdt-0.6b-v3",
            "parakeet-tdt-110m-en",
            "a-retired-model"
        ]
    );
}

#[test]
fn every_offered_model_is_pinned_to_one_https_archive_and_a_sha256() {
    let models = catalogue();
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "parakeet-tdt-0.6b-v3",
            "whisper-large-v3-turbo",
            "parakeet-tdt-110m-en",
            "moonshine-base-en"
        ]
    );
    for model in &models {
        assert!(
            model
                .url
                .starts_with("https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/"),
            "{}",
            model.url
        );
        assert!(model.url.ends_with(".tar.bz2"), "{}", model.url);
        assert_eq!(model.sha256.len(), 64);
        assert!(
            model
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(model.download_bytes > 0 && model.disk_bytes > model.download_bytes);
        // The licence asks for credit: who made it, the licence, and where to read it.
        for licence in ["CC BY 4.0", "MIT"] {
            if model.credit.contains(licence) {
                assert!(model.licence_url.starts_with("https://"), "{}", model.id);
            }
        }
        assert!(
            ["CC BY 4.0", "MIT"]
                .iter()
                .any(|licence| model.credit.contains(licence)),
            "{}",
            model.credit
        );
        assert!(!model.name.is_empty() && !model.detail.is_empty());
        // A tag is a word or two, never a sentence.
        let tag = model.tag.as_deref().unwrap_or_default();
        assert!(tag.split(' ').count() <= 2 && !tag.ends_with('.'), "{tag}");
        // A suggestion names another model on offer.
        if let Some(other) = &model.more_languages {
            assert!(ids.contains(&other.as_str()) && *other != model.id);
        }
        // A model's id names its directory.
        assert!(
            model
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        );
    }
    let engines: Vec<Engine> = models.iter().map(|model| model.engine).collect();
    assert_eq!(
        engines,
        [
            Engine::Transducer,
            Engine::Whisper,
            Engine::Transducer,
            Engine::Moonshine
        ]
    );
}

#[test]
fn a_manifest_from_before_other_kinds_is_a_transducer_and_each_kind_needs_its_own_files() {
    let root = tempfile::tempdir().unwrap();
    let old = place(
        root.path(),
        "parakeet-tdt-110m-en",
        "Parakeet English",
        &FILES,
    );
    let manifest = std::fs::read_to_string(old.join(MANIFEST)).unwrap();
    let without: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let mut without = without.as_object().unwrap().clone();
    without.remove("engine");
    std::fs::write(old.join(MANIFEST), serde_json::to_vec(&without).unwrap()).unwrap();

    place_as(
        root.path(),
        "whisper-large-v3-turbo",
        "Whisper",
        Engine::Whisper,
        Engine::Whisper.files(),
    );
    // A Moonshine missing one of its five files is not a model.
    place_as(
        root.path(),
        "moonshine-base-en",
        "Moonshine",
        Engine::Moonshine,
        &Engine::Moonshine.files()[1..],
    );
    let found: Vec<(String, Engine)> = installed(root.path())
        .into_iter()
        .map(|model| (model.id, model.engine))
        .collect();
    assert_eq!(
        found,
        [
            ("whisper-large-v3-turbo".to_string(), Engine::Whisper),
            ("parakeet-tdt-110m-en".to_string(), Engine::Transducer),
        ]
    );
}

#[test]
fn the_words_listened_for_are_cleaned_of_the_engines_syntax_kept_once_and_capped() {
    let asked = [
        "Ophelia",
        " Mack ",
        "mack",
        "Groq/Grok",
        ":5 Brix",
        "#hash @at",
        "New\nline",
        "nul\0byte",
        "O'Brien",
        "gpt-5.1",
        "...",
        "",
        "a word far too long to be a name anyone would ever say aloud",
    ];
    assert_eq!(
        hotwords(asked),
        [
            "Ophelia",
            "Mack",
            "Groq Grok",
            "5 Brix",
            "hash at",
            "New line",
            "nul byte",
            "O'Brien",
            "gpt-5.1"
        ]
    );
    let many: Vec<String> = (0..100).map(|n| format!("Name{n}")).collect();
    let kept = hotwords(many.iter().map(String::as_str));
    assert_eq!(kept.len(), MAX_HOTWORDS);
    assert_eq!(kept[0], "Name0", "the first asked are the ones kept");
}

#[test]
fn a_transducers_vocabulary_is_scored_so_a_word_splits_into_the_fewest_pieces() {
    let tokens = "<unk> 0\n▁t 1\n▁th 2\nin 3\n\nbroken\n▁Par 4\nake 5\na b c\n<blk> 6\n";
    let vocabulary = scored_vocabulary(tokens);
    let lines: Vec<(&str, f64)> = vocabulary
        .lines()
        .map(|line| {
            let (piece, score) = line.split_once(' ').unwrap();
            (piece, score.parse().unwrap())
        })
        .collect();
    // Only lines of exactly two fields, and never the blank: the encoder
    // ends the process on a line it cannot read.
    assert_eq!(
        lines.iter().map(|(piece, _)| *piece).collect::<Vec<_>>(),
        ["<unk>", "▁t", "▁th", "in", "▁Par", "ake"]
    );
    // Every piece costs about one, and an earlier merge a little less, so two
    // pieces always cost more than one whatever their place.
    for window in lines.windows(2) {
        assert!(window[0].1 > window[1].1);
    }
    let (best, worst) = (lines[0].1, lines.last().unwrap().1);
    assert!(best <= -1.0 && worst > -1.1);
    assert!(2.0 * best < worst);
}

#[test]
fn a_long_clip_is_heard_in_pieces_cut_where_it_is_quietest() {
    let rate = 1_000;
    // 40 seconds of sound with one quiet tenth of a second at 25 s.
    let mut samples = vec![0.5_f32; 40 * rate];
    for value in &mut samples[25 * rate..25 * rate + rate / 10] {
        *value = 0.0;
    }
    let cut = pieces(&samples, rate, Duration::from_secs(28));
    assert_eq!(cut.len(), 2);
    assert_eq!(cut[0].len(), 25 * rate + rate / 20);
    assert_eq!(
        cut.iter().map(|piece| piece.len()).sum::<usize>(),
        samples.len()
    );
    // A clip that fits is one piece.
    assert_eq!(
        pieces(&samples[..rate], rate, Duration::from_secs(28)).len(),
        1
    );
    // A minute with no quiet in it is still cut, at most 28 seconds a piece.
    let loud = vec![0.5_f32; 60 * rate];
    let cut = pieces(&loud, rate, Duration::from_secs(28));
    assert!(cut.iter().all(|piece| piece.len() <= 28 * rate));
    assert_eq!(
        cut.iter().map(|piece| piece.len()).sum::<usize>(),
        loud.len()
    );
}

#[test]
fn a_wav_is_read_at_its_own_rate_and_anything_else_is_refused() {
    let wav = pcm16_wav(&[0, 0x40, 0, 0xc0], 24_000);
    let heard = samples(&Clip {
        mime: "audio/wav".into(),
        bytes: wav,
    })
    .unwrap();
    assert_eq!(heard.rate, 24_000);
    assert_eq!(heard.values, [0.5, -0.5]);

    let mut stereo = pcm16_wav(&[0, 0], 16_000);
    stereo[22..24].copy_from_slice(&2u16.to_le_bytes());
    for (mime, bytes) in [
        ("audio/wav", stereo),
        ("audio/mp4", b"not an mp4".to_vec()),
        ("audio/ogg", pcm16_wav(&[0, 0], 16_000)),
    ] {
        assert_eq!(
            samples(&Clip {
                mime: mime.into(),
                bytes,
            }),
            Err(SpeechError::UnsupportedFormat(mime.into())),
            "{mime}"
        );
    }
}

#[test]
fn more_than_a_minute_is_refused_before_the_model_runs() {
    let pcm = vec![0; (MAX_SECONDS * RATE + 1) * 2];
    assert!(matches!(
        samples(&Clip {
            mime: "audio/wav".into(),
            bytes: pcm16_wav(&pcm, 16_000),
        }),
        Err(SpeechError::Engine(_))
    ));
}

#[test]
fn a_phones_aac_clip_decodes_to_its_whole_length() {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice/ask-mack.m4a"),
    )
    .unwrap();
    let heard = samples(&Clip {
        mime: "audio/mp4".into(),
        bytes,
    })
    .unwrap();
    assert_eq!(heard.rate, 16_000);
    let seconds = heard.values.len() as f32 / heard.rate as f32;
    assert!((2.3..2.6).contains(&seconds), "{seconds}");
    assert!(
        heard.values.iter().any(|value| value.abs() > 0.05),
        "it is speech, not silence"
    );
}

#[tokio::test]
async fn a_listener_never_speaks_and_hears_a_click_as_nothing_without_the_model() {
    let root = tempfile::tempdir().unwrap();
    place(root.path(), "click", "Click", &FILES);
    let local = listener(root.path(), "click");
    assert_eq!(local.speak("hello").await, Err(SpeechError::WrongJob));
    assert!(local.supports_live_input());
    assert_eq!(local.id().provider_id, PROVIDER_ID);
    assert_eq!(local.id().model_id, "click");
    // Under a tenth of a second: the engine would throw on it, and it holds no word.
    let heard = local
        .transcribe(Clip {
            mime: "audio/wav".into(),
            bytes: pcm16_wav(&[0; 3000], 16_000),
        })
        .await;
    assert_eq!(heard, Ok(String::new()));
}

#[tokio::test]
async fn a_damaged_model_is_refused_before_the_engine_could_throw_on_it() {
    let root = tempfile::tempdir().unwrap();
    let dir = place(root.path(), "damaged", "Damaged", &FILES);
    // The same size, so it is still listed; different bytes, so it is not the model.
    std::fs::write(dir.join(FILES[0]), b"MODEL").unwrap();
    let heard = listener(root.path(), "damaged")
        .transcribe(Clip {
            mime: "audio/wav".into(),
            bytes: pcm16_wav(&[0; 3200], 16_000),
        })
        .await;
    assert!(
        matches!(&heard, Err(SpeechError::Engine(sentence)) if sentence.contains("damaged")),
        "{heard:?}"
    );
}
