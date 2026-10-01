//! Draws one small picture through each provider whose key is in the
//! environment, on the real endpoints, and says what it cost and how long
//! it took. Keys are read and never printed.
//!
//! ```sh
//! OPENROUTER_API_KEY=... OPENAI_API_KEY=... GEMINI_API_KEY=... \
//!   cargo run -p hotline-core --example image_check [out-dir]
//! ```

use hotline_core::imagegen::{self, Aspect, Google, ImageGen, ImageRequest, OpenAi, OpenRouter};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "image-check".into()),
    );
    std::fs::create_dir_all(&out).expect("out dir");
    let key = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|key| !key.trim().is_empty())
    };
    let mut adapters: Vec<Arc<dyn ImageGen>> = Vec::new();
    if let Some(key) = key("OPENROUTER_API_KEY") {
        for id in [
            "openai/gpt-image-2.5-flare",
            "google/gemini-3.1-flash-image",
        ] {
            adapters.push(Arc::new(
                OpenRouter::new("https://openrouter.ai/api/v1", &key, imagegen::model(id)).unwrap(),
            ));
        }
    }
    if let Some(key) = key("OPENAI_API_KEY") {
        let model = imagegen::model("gpt-image-2.5-flare");
        adapters.push(Arc::new(
            OpenAi::new("openai", "https://api.openai.com/v1", Some(&key), model).unwrap(),
        ));
    }
    if let Some(key) = key("GEMINI_API_KEY") {
        let model = imagegen::model("gemini-3.1-flash-image");
        adapters.push(Arc::new(
            Google::new("https://generativelanguage.googleapis.com", &key, model).unwrap(),
        ));
    }
    if adapters.is_empty() {
        eprintln!("Set OPENROUTER_API_KEY, OPENAI_API_KEY or GEMINI_API_KEY.");
        std::process::exit(2);
    }
    let (prompt, aspect) = imagegen::styled(
        Some("avatar"),
        "a small lighthouse with a warm light",
        Aspect::Wide,
    )
    .unwrap();
    let request = ImageRequest {
        prompt,
        aspect,
        transparent: true,
        references: Vec::new(),
    };
    let mut failed = false;
    for adapter in adapters {
        let id = adapter.id();
        match adapter.generate(&request).await {
            Ok(image) => {
                let extension = image
                    .mime
                    .rsplit('/')
                    .next()
                    .unwrap_or("png")
                    .replace("svg+xml", "svg");
                let path = out.join(format!("{}.{extension}", id.to_string().replace('/', "__")));
                std::fs::write(&path, &image.bytes).expect("write");
                let cost = image.cost_usd.map_or_else(
                    || format!("~${:.3} (estimate)", adapter.estimate_usd(&request)),
                    |cost| format!("${cost:.4}"),
                );
                println!(
                    "{id}: {} bytes {} in {}ms, {cost}, transparent {} -> {}",
                    image.bytes.len(),
                    image.mime,
                    image.millis,
                    image.transparent,
                    path.display()
                );
            }
            Err(error) => {
                failed = true;
                println!("{id}: {error}");
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
