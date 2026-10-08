//! The desk's own speech models over the same door the window uses: nothing
//! is installed until the owner asks; a download is verified against its
//! pinned hash before any of it is unpacked, and only the model's own files
//! are taken from it; a download can be cancelled and a model removed; and an
//! installed model is offered for hearing, and chosen first by automatic.
//!
//! The archives here are small fakes served on localhost, which no test lets
//! the engine load. `HOTLINE_TEST_SPEECH_MODEL`, naming a directory with a
//! real sherpa-onnx Parakeet model in it, also runs
//! `a_real_model_installs_and_hears_what_was_said`, which hears the speech
//! fixtures through `voice.transcribe`.
mod common;

use axum::Router;
use axum::body::{Body, Bytes};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use hotline_core::voice::speech::local::Model;
use hotline_core::wire::Door;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// A client of the door, with its own command ids.
struct Client {
    socket: Socket,
    next: AtomicU64,
}

impl Client {
    async fn ask(&mut self, cmd: &str, params: Value) -> Value {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.socket
            .send(Message::text(
                json!({"id":id,"cmd":cmd,"params":params}).to_string(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let message = self.socket.next().await.unwrap().unwrap();
                let Ok(text) = message.to_text() else {
                    continue;
                };
                let frame: Value = serde_json::from_str(text).unwrap();
                if frame["id"] == id {
                    return frame;
                }
            }
        })
        .await
        .expect("the door did not answer")
    }

    /// The models, asked again until `done` says so, as the window polls a download.
    async fn models_until(&mut self, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let models = self.ask("voice.models", json!({})).await["result"]
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
}

fn model<'a>(models: &'a [Value], id: &str) -> &'a Value {
    models.iter().find(|one| one["id"] == id).unwrap()
}

/// A tar.bz2 of `entries`, named exactly as given, so an entry may name a
/// path tar's own checks would refuse to write. A `None` body is a symlink
/// to `/etc/passwd`.
fn archive(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(bzip2::write::BzEncoder::new(
        Vec::new(),
        bzip2::Compression::fast(),
    ));
    for (path, body) in entries {
        let mut header = tar::Header::new_gnu();
        header.as_gnu_mut().unwrap().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_mode(0o644);
        match body {
            Some(body) => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(body.len() as u64);
                header.set_cksum();
                builder.append(&header, *body).unwrap();
            }
            None => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_link_name("/etc/passwd").unwrap();
                header.set_cksum();
                builder.append(&header, std::io::empty()).unwrap();
            }
        }
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn offered(id: &str, url: String, bytes: &[u8]) -> Model {
    Model {
        id: id.into(),
        name: format!("Model {id}"),
        detail: "For the harness".into(),
        url,
        sha256: hex::encode(Sha256::digest(bytes)),
        download_bytes: bytes.len() as u64,
        disk_bytes: 1,
        credit: "Nobody, CC BY 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
    }
}

/// Serves `routes` on localhost; a route with no body sends a little and then stalls.
async fn serve(routes: Vec<(&'static str, Option<Vec<u8>>)>) -> String {
    let mut app = Router::new();
    for (path, body) in routes {
        app = app.route(
            path,
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
    let door = Door::bind(desk.log.clone(), "local-speech".into(), desk).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let (socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=local-speech"))
        .await
        .unwrap();
    Client {
        socket,
        next: AtomicU64::new(1),
    }
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
async fn a_model_is_downloaded_verified_unpacked_offered_and_removed_only_when_the_owner_asks() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let file = |name: &str| format!("model-{name}").into_bytes();
    let bodies: Vec<Vec<u8>> = FILES.iter().map(|name| file(name)).collect();
    let mut entries: Vec<(&str, Option<&[u8]>)> = vec![
        // Nothing an archive names chooses where a file lands.
        ("../../escaped.txt", Some(b"out")),
        ("sherpa/tokens.txt", None),
    ];
    let named: Vec<String> = FILES.iter().map(|name| format!("sherpa/{name}")).collect();
    for (path, body) in named.iter().zip(&bodies) {
        entries.push((path, Some(body)));
    }
    let good = archive(&entries);
    let short = archive(&entries[..entries.len() - 1]);
    let tampered = {
        let mut bytes = good.clone();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        bytes
    };
    let url = serve(vec![
        ("/good.tar.bz2", Some(good.clone())),
        ("/short.tar.bz2", Some(short.clone())),
        ("/tampered.tar.bz2", Some(tampered)),
        ("/stalled.tar.bz2", None),
    ])
    .await;
    let mut stalled = offered("stalled", format!("{url}/stalled.tar.bz2"), &good);
    stalled.download_bytes = 1 << 20;
    let mut client = open(
        &data,
        vec![
            offered("good", format!("{url}/good.tar.bz2"), &good),
            offered("short", format!("{url}/short.tar.bz2"), &short),
            // Pinned to the good archive; the server sends one with a bit flipped.
            offered("tampered", format!("{url}/tampered.tar.bz2"), &good),
            stalled,
        ],
    )
    .await;

    // Nothing is installed, offered or heard with until the owner asks.
    let models = client.models_until(|_| true).await;
    assert!(
        models.iter().all(|one| one["state"] == "available"),
        "{models:?}"
    );
    assert!(!data.join("speech-models").exists());
    let options = client.ask("capabilities.options", json!({})).await;
    assert!(!hearing_options(&options).contains(&"local".to_string()));
    let refused = client
        .ask(
            "voice.transcribe",
            json!({"mimeType":"audio/wav","data":STANDARD.encode(b"RIFF")}),
        )
        .await;
    assert_eq!(refused["ok"], false);

    // A good archive becomes a model, with only its own files.
    let asked = client
        .ask("voice.model_install", json!({"modelId":"good"}))
        .await;
    assert_eq!(asked["ok"], true, "{asked}");
    let models = client
        .models_until(|models| model(models, "good")["state"] == "installed")
        .await;
    assert!(model(&models, "good").get("error").is_none());
    let installed = data.join("speech-models").join("good");
    for (name, body) in FILES.iter().zip(&bodies) {
        let path = installed.join(name);
        assert!(
            !path.is_symlink(),
            "{name} is the archive's file, not its link"
        );
        assert_eq!(&std::fs::read(path).unwrap(), body);
    }
    let mut left: Vec<String> = std::fs::read_dir(data.join("speech-models"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["good"], "the download and its staging are gone");
    assert!(!root.path().join("escaped.txt").exists());
    assert!(!data.join("escaped.txt").exists());

    // Installed, it is offered for hearing and automatic picks it first.
    let options = client.ask("capabilities.options", json!({})).await;
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

    // An archive that is not the one pinned is thrown away before it is unpacked.
    client
        .ask("voice.model_install", json!({"modelId":"tampered"}))
        .await;
    let models = client
        .models_until(|models| model(models, "tampered").get("error").is_some())
        .await;
    assert_eq!(model(&models, "tampered")["state"], "available");
    assert!(
        model(&models, "tampered")["error"]
            .as_str()
            .unwrap()
            .contains("not the model Hotline expects")
    );
    assert!(!data.join("speech-models").join("tampered").exists());

    // A verified archive without every file of a model is not a model.
    client
        .ask("voice.model_install", json!({"modelId":"short"}))
        .await;
    let models = client
        .models_until(|models| model(models, "short").get("error").is_some())
        .await;
    assert!(
        model(&models, "short")["error"]
            .as_str()
            .unwrap()
            .contains("did not hold the model's files")
    );
    assert!(!data.join("speech-models").join("short").exists());

    // A download under way reports its progress and can be cancelled.
    client
        .ask("voice.model_install", json!({"modelId":"stalled"}))
        .await;
    let models = client
        .models_until(|models| model(models, "stalled")["receivedBytes"] == 4096)
        .await;
    assert_eq!(model(&models, "stalled")["state"], "downloading");
    let cancelled = client
        .ask("voice.model_cancel", json!({"modelId":"stalled"}))
        .await;
    assert_eq!(cancelled["ok"], true);
    let models = client
        .models_until(|models| model(models, "stalled")["state"] == "available")
        .await;
    assert!(model(&models, "stalled").get("error").is_none());
    assert!(!data.join("speech-models").join("stalled.download").exists());

    // Removed, it is gone from the disk and from the picker.
    let removed = client
        .ask("voice.model_remove", json!({"modelId":"good"}))
        .await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert_eq!(
        model(removed["result"].as_array().unwrap(), "good")["state"],
        "available"
    );
    assert!(!installed.exists());
    let options = client.ask("capabilities.options", json!({})).await;
    assert!(!hearing_options(&options).contains(&"local".to_string()));
}

#[tokio::test]
async fn a_real_model_installs_and_hears_what_was_said() {
    let Some(source) = std::env::var_os("HOTLINE_TEST_SPEECH_MODEL") else {
        eprintln!("HOTLINE_TEST_SPEECH_MODEL is not set; the model itself is not run.");
        return;
    };
    let source = Path::new(&source);
    let bodies: Vec<Vec<u8>> = FILES
        .iter()
        .map(|name| std::fs::read(source.join(name)).unwrap())
        .collect();
    let entries: Vec<(&str, Option<&[u8]>)> = FILES
        .iter()
        .zip(&bodies)
        .map(|(name, body)| (*name, Some(body.as_slice())))
        .collect();
    let packed = archive(&entries);
    let url = serve(vec![("/model.tar.bz2", Some(packed.clone()))]).await;
    let root = tempfile::tempdir().unwrap();
    let mut client = open(
        root.path(),
        vec![offered("real", format!("{url}/model.tar.bz2"), &packed)],
    )
    .await;
    client
        .ask("voice.model_install", json!({"modelId":"real"}))
        .await;
    client
        .models_until(|models| model(models, "real")["state"] == "installed")
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
            .ask("voice.transcribe", json!({"mimeType":mime,"data":data}))
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
