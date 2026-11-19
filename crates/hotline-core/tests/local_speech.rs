//! The desk's own speech models over the same door the window uses: nothing
//! is installed until the owner asks; each of a model's files is verified
//! against its pinned size and hash and kept under the desk's own name for
//! it; a download can be cancelled and a model removed; and an installed
//! model is offered for hearing, and chosen first by automatic.
//!
//! The files here are small fakes served on localhost, which no test lets
//! the engine load. `HOTLINE_TEST_SPEECH_MODEL`, naming a directory with a
//! real sherpa-onnx Parakeet model in it, with `--ignored`, also runs
//! `a_real_model_installs_and_hears_what_was_said`, which hears the speech
//! fixtures through `voice.transcribe`.
use crate::common;

use axum::Router;
use axum::body::{Body, Bytes};
use base64::{Engine, engine::general_purpose::STANDARD};
use common::Client;
use futures_util::StreamExt;
use hotline_core::wire::Door;
use hotline_voice::speech::local::{self, Model, Pinned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};

const FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// The models, asked again until `done` says so, as the window polls a download.
async fn models_until(client: &mut Client, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let models = client.call("voice.models", json!({})).await["result"]
            .as_array()
            .unwrap()
            .clone();
        if done(&models) {
            return models;
        }
        assert!(
            Instant::now() < deadline,
            "the models never settled: {models:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn model<'a>(models: &'a [Value], id: &str) -> &'a Value {
    models.iter().find(|one| one["id"] == id).unwrap()
}

/// The model `id`, offered as `files`: for each, the desk's name for it,
/// where under `url` it is served, and the bytes it is pinned to.
fn offered(id: &str, url: &str, files: &[(&str, &str, &[u8])]) -> Model {
    Model {
        id: id.into(),
        name: format!("Model {id}"),
        tag: None,
        detail: "For the harness".into(),
        engine: local::Engine::Transducer,
        files: files
            .iter()
            .map(|(name, path, body)| Pinned {
                name: (*name).into(),
                url: format!("{url}/{path}"),
                bytes: body.len() as u64,
                sha256: hex::encode(Sha256::digest(body)),
            })
            .collect(),
        credit: "Nobody, CC BY 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
        more_languages: None,
    }
}

/// Serves `routes` on localhost; a route with no body sends a little and then stalls.
async fn serve(routes: Vec<(String, Option<Vec<u8>>)>) -> String {
    let mut app = Router::new();
    for (path, body) in routes {
        app = app.route(
            &path,
            axum::routing::get(move || {
                let body = body.clone();
                async move {
                    match body {
                        Some(body) => Body::from(body),
                        None => Body::from_stream(
                            futures_util::stream::once(async {
                                Ok::<_, std::io::Error>(Bytes::from(vec![7u8; 4096]))
                            })
                            .chain(futures_util::stream::pending()),
                        ),
                    }
                }
            }),
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

async fn open(root: &Path, models: Vec<Model>) -> Client {
    let desk = std::sync::Arc::new(
        hotline_core::desk::Desk::open_with_speech_models(root, common::store(), models).unwrap(),
    );
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), "local-speech".into(), desk, parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    Client::connect(port, "local-speech").await
}

fn hearing_options(options: &Value) -> Vec<String> {
    options["result"]["stt"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|provider| provider["providerId"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_model_is_downloaded_verified_offered_and_removed_only_when_the_owner_asks() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let bodies: Vec<Vec<u8>> = FILES
        .iter()
        .map(|name| format!("model-{name}").into_bytes())
        .collect();
    let pinned: Vec<(&str, &str, &[u8])> = FILES
        .iter()
        .zip(&bodies)
        .map(|(name, body)| (*name, *name, body.as_slice()))
        .collect();
    let mut routes = Vec::new();
    for (name, body) in FILES.iter().zip(&bodies) {
        routes.push((format!("/good/{name}"), Some(body.clone())));
        let mut flipped = body.clone();
        flipped[0] ^= 1;
        routes.push((format!("/tampered/{name}"), Some(flipped)));
        routes.push((format!("/short/{name}"), Some(body[1..].to_vec())));
        routes.push((format!("/stalled/{name}"), None));
    }
    let url = serve(routes).await;
    let mut escaped = pinned.clone();
    // Nothing a catalogue names chooses where a file lands but the engine.
    escaped[0].0 = "../../escaped.txt";
    let mut stalled = offered("stalled", &format!("{url}/stalled"), &pinned);
    stalled.files[0].bytes = 1 << 20;
    let mut client = open(
        &data,
        vec![
            offered("good", &format!("{url}/good"), &pinned),
            // Pinned to the good files; the server sends one with a bit
            // flipped, or one a byte short.
            offered("tampered", &format!("{url}/tampered"), &pinned),
            offered("short", &format!("{url}/short"), &pinned),
            offered("escaped", &format!("{url}/good"), &escaped),
            stalled,
        ],
    )
    .await;

    // Nothing is installed, offered or heard with until the owner asks.
    let models = models_until(&mut client, |_| true).await;
    assert!(
        models.iter().all(|one| one["state"] == "available"),
        "{models:?}"
    );
    assert!(!data.join("speech-models").exists());
    let options = client.call("capabilities.options", json!({})).await;
    assert!(!hearing_options(&options).contains(&"local".to_string()));
    let refused = client
        .call(
            "voice.transcribe",
            json!({"mimeType":"audio/wav","data":STANDARD.encode(b"RIFF")}),
        )
        .await;
    assert_eq!(refused["ok"], false);

    // Good files become a model, under the desk's names for them.
    let asked = client
        .call("voice.model_install", json!({"modelId":"good"}))
        .await;
    assert_eq!(asked["ok"], true, "{asked}");
    let models = models_until(&mut client, |models| {
        model(models, "good")["state"] == "installed"
    })
    .await;
    assert!(model(&models, "good").get("error").is_none());
    let installed = data.join("speech-models").join("good");
    for (name, body) in FILES.iter().zip(&bodies) {
        let path = installed.join(name);
        assert_eq!(&std::fs::read(path).unwrap(), body);
    }
    let mut left: Vec<String> = std::fs::read_dir(data.join("speech-models"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["good"], "the download and its staging are gone");
    assert!(!root.path().join("escaped.txt").exists());

    // Installed, it is offered for hearing and automatic picks it first.
    let options = client.call("capabilities.options", json!({})).await;
    assert_eq!(hearing_options(&options), ["local"]);
    assert_eq!(
        options["result"]["stt"]["options"][0]["models"][0],
        json!({"id":"good","label":"Model good"})
    );
    assert_eq!(options["result"]["stt"]["automatic"]["providerId"], "local");
    assert_eq!(
        options["result"]["stt"]["automatic"]["providerName"],
        "On the desk"
    );

    // A file that is not the one pinned, or a name that is not the
    // engine's, is no model, and nothing of it is kept.
    for id in ["tampered", "short", "escaped"] {
        client
            .call("voice.model_install", json!({"modelId":id}))
            .await;
        let models = models_until(&mut client, |models| {
            model(models, id).get("error").is_some()
        })
        .await;
        assert_eq!(model(&models, id)["state"], "available", "{id}");
        assert!(
            model(&models, id)["error"]
                .as_str()
                .unwrap()
                .contains("not the model Hotline expects"),
            "{id}"
        );
        assert!(!data.join("speech-models").join(id).exists(), "{id}");
        assert!(
            !data
                .join("speech-models")
                .join(format!("{id}.unpacking"))
                .exists(),
            "{id}"
        );
    }
    assert!(!root.path().join("escaped.txt").exists());
    assert!(!data.join("escaped.txt").exists());

    // A download under way reports its progress and can be cancelled.
    client
        .call("voice.model_install", json!({"modelId":"stalled"}))
        .await;
    let models = models_until(&mut client, |models| {
        model(models, "stalled")["receivedBytes"] == 4096
    })
    .await;
    assert_eq!(model(&models, "stalled")["state"], "downloading");
    let cancelled = client
        .call("voice.model_cancel", json!({"modelId":"stalled"}))
        .await;
    assert_eq!(cancelled["ok"], true);
    let models = models_until(&mut client, |models| {
        model(models, "stalled")["state"] == "available"
    })
    .await;
    assert!(model(&models, "stalled").get("error").is_none());
    assert!(
        !data
            .join("speech-models")
            .join("stalled.unpacking")
            .exists()
    );

    // Removed, it is gone from the disk and from the picker.
    let removed = client
        .call("voice.model_remove", json!({"modelId":"good"}))
        .await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert_eq!(
        model(removed["result"].as_array().unwrap(), "good")["state"],
        "available"
    );
    assert!(!installed.exists());
    let options = client.call("capabilities.options", json!({})).await;
    assert!(!hearing_options(&options).contains(&"local".to_string()));
}

#[tokio::test]
async fn a_whisper_models_files_are_kept_under_the_desks_own_names() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let url = serve(
        [
            ("turbo-encoder.int8.onnx", b"encoder".as_slice()),
            ("turbo-decoder.int8.onnx", b"decoder"),
            ("turbo-tokens.txt", b"tokens"),
        ]
        .into_iter()
        .map(|(name, body)| (format!("/{name}"), Some(body.to_vec())))
        .collect(),
    )
    .await;
    let mut whisper = offered(
        "whisper",
        &url,
        &[
            ("encoder.int8.onnx", "turbo-encoder.int8.onnx", b"encoder"),
            ("decoder.int8.onnx", "turbo-decoder.int8.onnx", b"decoder"),
            ("tokens.txt", "turbo-tokens.txt", b"tokens"),
        ],
    );
    whisper.engine = local::Engine::Whisper;
    whisper.tag = Some("99 languages".into());
    let mut english = offered("english", &url, &[("tokens.txt", "missing.txt", b"")]);
    english.more_languages = Some("whisper".into());
    let mut client = open(&data, vec![whisper, english]).await;

    let models = models_until(&mut client, |_| true).await;
    assert_eq!(model(&models, "whisper")["tag"], "99 languages");
    assert_eq!(model(&models, "english")["moreLanguages"], "whisper");
    assert!(model(&models, "english").get("tag").is_none());

    client
        .call("voice.model_install", json!({"modelId":"whisper"}))
        .await;
    models_until(&mut client, |models| {
        model(models, "whisper")["state"] == "installed"
    })
    .await;
    let dir = data.join("speech-models").join("whisper");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "decoder.int8.onnx",
            "encoder.int8.onnx",
            "model.json",
            "tokens.txt"
        ]
    );
    assert_eq!(
        std::fs::read(dir.join("encoder.int8.onnx")).unwrap(),
        b"encoder"
    );
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(dir.join("model.json")).unwrap()).unwrap();
    assert_eq!(manifest["engine"], "whisper");
}

#[tokio::test]
#[ignore = "needs a Parakeet model directory in HOTLINE_TEST_SPEECH_MODEL"]
async fn a_real_model_installs_and_hears_what_was_said() {
    let source = std::env::var_os("HOTLINE_TEST_SPEECH_MODEL")
        .expect("set HOTLINE_TEST_SPEECH_MODEL to a sherpa-onnx Parakeet model directory");
    let source = Path::new(&source);
    let bodies: Vec<Vec<u8>> = FILES
        .iter()
        .map(|name| std::fs::read(source.join(name)).unwrap())
        .collect();
    let url = serve(
        FILES
            .iter()
            .zip(&bodies)
            .map(|(name, body)| (format!("/{name}"), Some(body.clone())))
            .collect(),
    )
    .await;
    let pinned: Vec<(&str, &str, &[u8])> = FILES
        .iter()
        .zip(&bodies)
        .map(|(name, body)| (*name, *name, body.as_slice()))
        .collect();
    let root = tempfile::tempdir().unwrap();
    let mut client = open(root.path(), vec![offered("real", &url, &pinned)]).await;
    client
        .call("voice.model_install", json!({"modelId":"real"}))
        .await;
    models_until(&mut client, |models| {
        model(models, "real")["state"] == "installed"
    })
    .await;

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice");
    for (file, mime) in [
        ("ask-mack.wav", "audio/wav"),
        ("ask-mack.m4a", "audio/mp4"),
        ("ask-mack.wav", "audio/wav"),
    ] {
        let data = STANDARD.encode(std::fs::read(fixtures.join(file)).unwrap());
        let started = Instant::now();
        let heard = client
            .call("voice.transcribe", json!({"mimeType":mime,"data":data}))
            .await;
        let text = heard["result"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        eprintln!(
            "{file}: {:?} in {}ms",
            heard["result"]["text"],
            started.elapsed().as_millis()
        );
        // Mack or Mac: the fixture's voice says them alike.
        assert!(text.contains("mac") && text.contains("failing"), "{heard}");
    }
}
