//! Speaks a sentence through each provider whose key is in the environment,
//! then has each provider transcribe what was spoken, on the real endpoints
//! and through `resolve`, the code the desk runs. A voice that reads an
//! instruction aloud, or a shape that has drifted, shows as different words.
//! The time to first byte of every call is the desk's own log line, on stderr.
//!
//! OPENAI_API_KEY GEMINI_API_KEY OPENROUTER_API_KEY GROQ_API_KEY MISTRAL_API_KEY
//! cargo run -p hotline-core --example speech_check

#[cfg(unix)]
use hotline_core::{
    credentials::FileStore,
    log::Log,
    vault::Vault,
    voice::{
        settings::{Choice, VoiceSettings},
        speech::{Clip, resolve},
    },
};
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
const KEYS: &[(&str, &str)] = &[
    ("openai", "OPENAI_API_KEY"),
    ("google", "GEMINI_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
    ("groq", "GROQ_API_KEY"),
    ("mistral", "MISTRAL_API_KEY"),
];

#[cfg(unix)]
const SENTENCE: &str = "Handing that to Mack.";

#[cfg(unix)]
fn pick(provider: &str) -> Option<Choice> {
    Some(Choice {
        provider_id: provider.into(),
        model_id: None,
        voice: None,
        effort: None,
    })
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A scratch data directory: nothing here touches a real desk's vault.
    let root = tempfile::tempdir()?;
    let vault = Vault::open_with_store(
        root.path(),
        Log::open(root.path()),
        Arc::new(FileStore::open(root.path())?),
    )?;
    let mut connected = Vec::new();
    for (provider, variable) in KEYS {
        if let Ok(key) = std::env::var(variable) {
            vault.create(provider, provider, key.trim())?;
            connected.push(*provider);
        }
    }
    if connected.is_empty() {
        eprintln!(
            "Set at least one of: {}",
            KEYS.iter().map(|(_, v)| *v).collect::<Vec<_>>().join(" ")
        );
        std::process::exit(2);
    }

    let mut spoken: Option<Clip> = None;
    for provider in &connected {
        let settings = VoiceSettings {
            tts: pick(provider),
            ..VoiceSettings::default()
        };
        let voice = match resolve(&vault, &settings) {
            Ok(set) => set.tts,
            Err(reason) => {
                println!("{provider}: does not speak: {reason}");
                continue;
            }
        };
        match voice.speak(SENTENCE).await {
            Ok(clip) => {
                println!(
                    "{provider}: spoke {SENTENCE:?} as {} bytes of {}",
                    clip.bytes.len(),
                    clip.mime
                );
                if clip.mime == "audio/wav" {
                    spoken.get_or_insert(clip);
                }
            }
            Err(error) => println!("{provider}: could not speak: {error}"),
        }
    }

    let Some(clip) = spoken else {
        println!("No provider spoke a WAV, so there is nothing to transcribe.");
        return Ok(());
    };
    for provider in &connected {
        let settings = VoiceSettings {
            stt: pick(provider),
            ..VoiceSettings::default()
        };
        match resolve(&vault, &settings) {
            Ok(set) => match set.stt.transcribe(clip.clone()).await {
                Ok(words) => println!("{provider}: heard {words:?}"),
                Err(error) => println!("{provider}: could not hear: {error}"),
            },
            Err(reason) => println!("{provider}: does not hear: {reason}"),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("Run this on Linux or macOS.");
}
