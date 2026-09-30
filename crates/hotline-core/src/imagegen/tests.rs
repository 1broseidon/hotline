//! The adapters against mock servers on localhost: what they send, what they
//! make of the answers, and that a failure names only the provider and
//! status. No test here reaches the network.

use super::providers::model;
use super::*;
use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use std::sync::Mutex;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRpicture";
const SECRET: &str = "sk-test-never-shown";

struct Seen {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

type Requests = Arc<Mutex<Vec<Seen>>>;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A server that answers every request the same way, and remembers them.
async fn answering(status: StatusCode, body: Vec<u8>) -> (String, Requests, Server) {
    let requests = Requests::default();
    let seen = requests.clone();
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let seen = seen.clone();
        let body = body.clone();
        async move {
            seen.lock().unwrap().push(Seen {
                path: uri.path_and_query().unwrap().to_string(),
                headers,
                body: bytes.to_vec(),
            });
            (status, [(header::CONTENT_TYPE, "application/json")], body)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, requests, Server(task))
}

fn ask(prompt: &str) -> ImageRequest {
    ImageRequest {
        prompt: prompt.into(),
        ..ImageRequest::default()
    }
}

fn reference() -> Reference {
    Reference {
        mime: "image/png".into(),
        bytes: PNG.to_vec(),
    }
}

#[tokio::test]
async fn openrouter_sends_one_request_shape_and_reads_the_cost() {
    let answer = json!({"data": [{"b64_json": STANDARD.encode(PNG), "media_type": "image/png"}], "usage": {"cost": 0.0064}});
    let (url, seen, _server) = answering(StatusCode::OK, answer.to_string().into_bytes()).await;
    let adapter = OpenRouter::new(&url, SECRET, model("openai/gpt-image-2.5-flare")).unwrap();
    let request = ImageRequest {
        prompt: "an otter in a hard hat".into(),
        aspect: Aspect::Wide,
        transparent: true,
        references: vec![reference()],
    };

    let image = adapter.generate(&request).await.unwrap();

    assert_eq!(image.bytes, PNG);
    assert_eq!(image.mime, "image/png");
    assert!(image.transparent);
    assert_eq!(image.cost_usd, Some(0.0064));
    assert_eq!(
        image.id.to_string(),
        "openrouter/openai/gpt-image-2.5-flare"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/images");
    assert_eq!(seen[0].headers["authorization"], format!("Bearer {SECRET}"));
    let body = seen[0].json();
    assert_eq!(body["model"], "openai/gpt-image-2.5-flare");
    assert_eq!(body["prompt"], "an otter in a hard hat");
    assert_eq!(body["aspect_ratio"], "16:9");
    assert_eq!(body["quality"], "low");
    assert_eq!(body["background"], "transparent");
    assert_eq!(body["output_format"], "png");
    let url = body["input_references"][0]["image_url"]["url"]
        .as_str()
        .unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
}

#[tokio::test]
async fn transparency_is_asked_for_only_where_a_model_has_it() {
    let answer = json!({"data": [{"b64_json": STANDARD.encode(PNG)}]});
    let (url, seen, _server) = answering(StatusCode::OK, answer.to_string().into_bytes()).await;
    let adapter = OpenRouter::new(&url, SECRET, model("google/gemini-3.1-flash-image")).unwrap();
    let image = adapter
        .generate(&ImageRequest {
            transparent: true,
            ..ask("a lighthouse")
        })
        .await
        .unwrap();
    assert!(!image.transparent);
    assert_eq!(image.cost_usd, None);
    let body = seen.lock().unwrap()[0].json();
    assert!(body.get("background").is_none() && body.get("quality").is_none());
}

#[tokio::test]
async fn openai_generates_from_words_and_edits_with_references() {
    let answer = json!({"data": [{"b64_json": STANDARD.encode(PNG)}]});
    let (url, seen, _server) = answering(StatusCode::OK, answer.to_string().into_bytes()).await;
    let adapter = OpenAi::new("openai", &url, Some(SECRET), model("gpt-image-2.5-flare")).unwrap();

    let image = adapter
        .generate(&ImageRequest {
            aspect: Aspect::Portrait,
            transparent: true,
            ..ask("a clementine in glasses")
        })
        .await
        .unwrap();
    assert!(image.transparent);
    assert_eq!(image.cost_usd, None);
    adapter
        .generate(&ImageRequest {
            references: vec![reference(), reference()],
            ..ask("make this logo blue")
        })
        .await
        .unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].path, "/images/generations");
    let body = seen[0].json();
    assert_eq!(body["size"], "1024x1536");
    assert_eq!(body["n"], 1);
    assert_eq!(body["background"], "transparent");
    assert_eq!(seen[1].path, "/images/edits");
    let content_type = seen[1].headers["content-type"].to_str().unwrap();
    assert!(content_type.starts_with("multipart/form-data; boundary=hotline-"));
    let form = String::from_utf8_lossy(&seen[1].body);
    assert!(form.contains("name=\"prompt\"\r\n\r\nmake this logo blue\r\n"));
    assert_eq!(form.matches("name=\"image[]\"").count(), 2);
    assert!(form.contains("Content-Type: image/png"));
}

#[tokio::test]
async fn google_sends_the_key_as_a_header_and_finds_the_picture_among_parts() {
    let answer = json!({"candidates": [{"content": {"parts": [
        {"text": "Here is your image."},
        {"inlineData": {"mimeType": "image/png", "data": STANDARD.encode(PNG)}}
    ]}}]});
    let (url, seen, _server) = answering(StatusCode::OK, answer.to_string().into_bytes()).await;
    let adapter = Google::new(&url, SECRET, model("gemini-3.1-flash-image")).unwrap();

    let image = adapter
        .generate(&ImageRequest {
            references: vec![reference()],
            ..ask("a wren in headphones")
        })
        .await
        .unwrap();

    assert_eq!(image.bytes, PNG);
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0].path,
        "/v1beta/models/gemini-3.1-flash-image:generateContent"
    );
    assert!(!seen[0].path.contains(SECRET));
    assert_eq!(seen[0].headers["x-goog-api-key"], SECRET);
    let body = seen[0].json();
    assert_eq!(
        body["contents"][0]["parts"][0]["text"],
        "a wren in headphones"
    );
    assert_eq!(
        body["contents"][0]["parts"][1]["inline_data"]["mime_type"],
        "image/png"
    );
    assert_eq!(body["generationConfig"]["responseModalities"][0], "IMAGE");
    assert_eq!(
        body["generationConfig"]["imageConfig"]["aspectRatio"],
        "1:1"
    );
}

#[tokio::test]
async fn a_refusal_names_the_provider_and_status_and_never_the_body() {
    let body = json!({"error": {"message": "your prompt 'an otter' was flagged"}});
    let (url, _seen, _server) =
        answering(StatusCode::BAD_REQUEST, body.to_string().into_bytes()).await;
    let adapter = OpenRouter::new(&url, SECRET, model("openai/gpt-image-2.5-flare")).unwrap();
    let error = adapter.generate(&ask("an otter")).await.unwrap_err();
    assert_eq!(
        error,
        ImageError::Refused {
            provider_id: "openrouter".into(),
            status: 400
        }
    );
    let said = error.to_string();
    assert!(!said.contains("flagged") && !said.contains(SECRET));
}

#[tokio::test]
async fn an_answer_that_isnt_a_picture_is_malformed() {
    for answer in [
        json!({"data": []}).to_string(),
        json!({"data": [{"b64_json": "not base64!"}]}).to_string(),
        json!({"data": [{"b64_json": STANDARD.encode(b"{\"json\": true}")}]}).to_string(),
        "<html>gateway</html>".to_string(),
    ] {
        let (url, _seen, _server) = answering(StatusCode::OK, answer.into_bytes()).await;
        let adapter = OpenRouter::new(&url, SECRET, model("openai/gpt-image-2.5-flare")).unwrap();
        assert_eq!(
            adapter.generate(&ask("x")).await.unwrap_err(),
            ImageError::Malformed {
                provider_id: "openrouter".into()
            }
        );
    }
}

#[tokio::test]
async fn nothing_is_sent_for_a_request_the_model_cant_take() {
    let (url, seen, _server) = answering(StatusCode::OK, Vec::new()).await;
    let adapter = Google::new(&url, SECRET, model("gemini-3.1-flash-image")).unwrap();
    assert_eq!(
        adapter.generate(&ask("   ")).await.unwrap_err(),
        ImageError::EmptyPrompt
    );
    let many = ImageRequest {
        references: vec![reference(); 9],
        ..ask("x")
    };
    assert_eq!(
        adapter.generate(&many).await.unwrap_err(),
        ImageError::TooManyReferences { max: 8 }
    );
    let gif = ImageRequest {
        references: vec![Reference {
            mime: "image/gif".into(),
            bytes: vec![1],
        }],
        ..ask("x")
    };
    assert_eq!(
        adapter.generate(&gif).await.unwrap_err(),
        ImageError::UnsupportedReference("image/gif".into())
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn an_estimate_counts_references_and_rounds_up() {
    let flare = model("openai/gpt-image-2.5-flare");
    assert_eq!(flare.estimate_usd(&ask("x")), 0.02);
    let with_two = ImageRequest {
        references: vec![reference(), reference()],
        ..ask("x")
    };
    assert!((flare.estimate_usd(&with_two) - 0.04).abs() < 1e-9);
}

#[tokio::test]
async fn a_reported_cost_of_zero_counts_as_unreported() {
    // Gemini through OpenRouter answered `"cost": 0` on 30 Sep; it isn't free.
    let answer = json!({"data": [{"b64_json": STANDARD.encode(PNG)}], "usage": {"cost": 0}});
    let (url, _seen, _server) = answering(StatusCode::OK, answer.to_string().into_bytes()).await;
    let adapter = OpenRouter::new(&url, SECRET, model("google/gemini-3.1-flash-image")).unwrap();
    assert_eq!(adapter.generate(&ask("x")).await.unwrap().cost_usd, None);
}
