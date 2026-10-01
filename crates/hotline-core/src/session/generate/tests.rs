use super::*;
use crate::contract::{McpPolicy, Persona, PolicyMode};
use crate::driver::CapabilityEpoch;
use crate::imagegen::{Aspect, Image, ImageError, ImageGen, ImageId};
use crate::log::{Log, StreamId};
use crate::mcp::server::TeammateTools;
use crate::session::ProviderKeys;
use async_trait::async_trait;
use rig::tool::{ToolContext, ToolSet};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

struct NoKeys;

impl ProviderKeys for NoKeys {
    fn provider_auth(&self) -> HashMap<String, crate::session::ProviderAuth> {
        HashMap::new()
    }
}

struct Fake {
    subscription: bool,
    model: &'static str,
    estimate: f64,
    cost: Option<f64>,
    error: Option<ImageError>,
    max_references: usize,
    bytes: Option<Vec<u8>>,
    requests: Mutex<Vec<ImageRequest>>,
    entered: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
    preparing: Option<Arc<Notify>>,
    prepared: Option<Arc<Notify>>,
}

impl Fake {
    fn new(model: &'static str) -> Arc<Self> {
        Arc::new(Self {
            subscription: false,
            model,
            estimate: 0.01,
            cost: Some(0.006),
            error: None,
            max_references: 16,
            bytes: None,
            requests: Mutex::new(Vec::new()),
            entered: None,
            release: None,
            preparing: None,
            prepared: None,
        })
    }
}

#[async_trait]
impl ImageGen for Fake {
    fn subscription(&self) -> bool {
        self.subscription
    }
    fn id(&self) -> ImageId {
        ImageId {
            provider_id: "fake".into(),
            model_id: self.model.into(),
        }
    }

    fn transparent(&self) -> bool {
        true
    }
    fn max_references(&self) -> usize {
        self.max_references
    }
    fn estimate_usd(&self, _: &ImageRequest) -> f64 {
        self.estimate
    }

    async fn generate_checked(
        &self,
        request: &ImageRequest,
        before_send: &(dyn Fn() -> Result<(), ImageError> + Send + Sync),
    ) -> Result<Image, ImageError> {
        if let Some(preparing) = &self.preparing {
            preparing.notify_one();
        }
        if let Some(prepared) = &self.prepared {
            prepared.notified().await;
        }
        before_send()?;
        self.generate(request).await
    }

    async fn generate(&self, request: &ImageRequest) -> Result<Image, ImageError> {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(entered) = &self.entered {
            entered.notify_one();
        }
        if let Some(release) = &self.release {
            release.notified().await;
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(Image {
            mime: "image/png".into(),
            bytes: self.bytes.clone().unwrap_or_else(png),
            id: self.id(),
            transparent: request.transparent,
            cost_usd: self.cost,
            millis: 10,
        })
    }
}

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([30, 80, 120, 0]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// The temp dir as the desk reports it: resolved, as a saved file's path is
/// (macOS's `/var` is `/private/var`, Windows adds `\\?\`).
fn real(dir: &tempfile::TempDir) -> std::path::PathBuf {
    std::fs::canonicalize(dir.path()).unwrap()
}

fn room() -> (tempfile::TempDir, Arc<Room>, TeammateTools) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let log = Log::open(dir.path().join("data"));
    let persona = Persona {
        node: None,
        id: "ada".into(),
        name: "Ada".into(),
        goal: "Keep the harbour running".into(),
        avatar: None,
        team: None,
        backend_id: "hotline".into(),
        cwd: workspace.to_string_lossy().into_owned(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::None,
            server_ids: Vec::new(),
        },
        skill_policy: Default::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        computer: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1,
        updated_at: 1,
    };
    let mut record = serde_json::to_value(persona).unwrap();
    record["kind"] = json!("persona");
    log.append(&StreamId::Room, &record).unwrap();
    let room = Room::new(log, Arc::new(NoKeys));
    let tools = TeammateTools::new(&room, "ada");
    (dir, room, tools)
}

fn settings(room: &Room, day: f64) {
    room.log()
        .append(
            &StreamId::Room,
            &json!({
                "kind": "setting", "id": "spending", "value": { "dayUsd": day, "monthUsd": 20.0 }
            }),
        )
        .unwrap();
}

fn install(room: &Room, primary: Arc<Fake>, fallback: Option<Arc<Fake>>) {
    room.set_image_generators(ImageSet {
        primary,
        fallback: fallback.map(|adapter| adapter as Arc<dyn ImageGen>),
    });
}

fn attachments(room: &Room) -> Vec<Value> {
    room.log()
        .load(&StreamId::Tape("ada".into()))
        .into_iter()
        .filter_map(|event| event.get("attachments").cloned())
        .flat_map(|value| value.as_array().unwrap().clone())
        .collect()
}

#[tokio::test]
async fn subscription_images_report_unknown_cost_without_touching_dollar_spending() {
    let (dir, room, tools) = room();
    let mut fake = Fake::new("subscription-image");
    let provider = Arc::get_mut(&mut fake).unwrap();
    provider.subscription = true;
    provider.estimate = 0.0;
    provider.cost = None;
    install(&room, fake, None);
    room.spending
        .reserve(&SpendingSettings::default(), 1.0)
        .unwrap()
        .charge(1.0)
        .unwrap();
    let ledger_path = room.log().root().join("spending.json");
    let before = std::fs::read(&ledger_path).unwrap();
    for cap in [0.0, 0.5] {
        settings(&room, cap);
        let result: Value = serde_json::from_str(
            &tools
                .call("generate_image", &json!({"prompt":"draw"}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(result["costUsd"].is_null());
        assert_eq!(result["billing"], "subscription");
        assert!(
            Path::new(result["path"].as_str().unwrap()).starts_with(real(&dir).join("workspace"))
        );
        assert_eq!(
            std::fs::read(result["path"].as_str().unwrap()).unwrap(),
            png()
        );
        assert_eq!(std::fs::read(&ledger_path).unwrap(), before);
    }
    assert_eq!(attachments(&room).len(), 2);
}

#[tokio::test]
async fn a_subscription_refusal_never_tries_a_paid_fallback() {
    let (_dir, room, tools) = room();
    let mut fake = Fake::new("subscription-image");
    let provider = Arc::get_mut(&mut fake).unwrap();
    provider.subscription = true;
    provider.error = Some(ImageError::Refused {
        provider_id: "openai-codex".into(),
        status: 403,
    });
    let paid = Fake::new("paid");
    install(&room, fake, Some(paid.clone()));
    let error = tools
        .call("generate_image", &json!({"prompt":"draw"}))
        .await
        .unwrap_err();
    assert!(error.contains("403"));
    assert!(paid.requests.lock().unwrap().is_empty());
    assert!(!room.log().root().join("spending.json").exists());
}

#[tokio::test]
async fn subscription_images_cannot_read_references_outside_the_workspace() {
    let (dir, room, tools) = room();
    std::fs::write(dir.path().join("outside.png"), png()).unwrap();
    let mut fake = Fake::new("subscription-image");
    Arc::get_mut(&mut fake).unwrap().subscription = true;
    install(&room, fake.clone(), None);
    let error = tools
        .call(
            "generate_image",
            &json!({
                "prompt":"edit", "references":["../outside.png"]
            }),
        )
        .await;
    assert!(error.is_err());
    assert!(fake.requests.lock().unwrap().is_empty());
    assert!(attachments(&room).is_empty());
}

#[tokio::test]
async fn revocation_during_subscription_refresh_prevents_dispatch_and_publication() {
    let (dir, room, tools) = room();
    let epoch = CapabilityEpoch::default();
    let tools = tools.with_capability(epoch.lease());
    let preparing = Arc::new(Notify::new());
    let prepared = Arc::new(Notify::new());
    let mut fake = Fake::new("subscription-image");
    let provider = Arc::get_mut(&mut fake).unwrap();
    provider.subscription = true;
    provider.preparing = Some(preparing.clone());
    provider.prepared = Some(prepared.clone());
    let fallback = Fake::new("paid");
    install(&room, fake.clone(), Some(fallback.clone()));
    let call = tokio::spawn(async move {
        tools
            .call(
                "generate_image",
                &json!({"prompt":"draw", "name":"revoked"}),
            )
            .await
    });
    preparing.notified().await;
    epoch.invalidate();
    prepared.notify_one();
    let error = call.await.unwrap().unwrap_err();
    assert!(error.contains("revoked"), "{error}");
    assert!(fake.requests.lock().unwrap().is_empty());
    assert!(fallback.requests.lock().unwrap().is_empty());
    assert!(attachments(&room).is_empty());
    assert!(!dir.path().join("workspace/revoked.png").exists());
    assert!(!room.log().root().join("spending.json").exists());
}

#[tokio::test]
async fn generation_through_rig_keeps_the_image_in_the_workspace_and_chat_and_charges_it() {
    let (dir, room, tools) = room();
    let fake = Fake::new("fake-image");
    install(&room, fake.clone(), None);
    settings(&room, 0.011);
    let set = ToolSet::from_dynamic_tools(tools.as_dynamic());
    let result = set.execute("generate_image", json!({
        "prompt": "a lighthouse at dusk", "aspect": "16:9", "transparent": true, "name": "hero.jpg"
    }).to_string(), &mut ToolContext::new()).await;
    assert!(result.is_success(), "{result:?}");
    let result: Value = serde_json::from_str(result.output().as_text().unwrap()).unwrap();
    assert_eq!(result["path"], json!(real(&dir).join("workspace/hero.png")));
    assert_eq!(result["model"], "fake-image");
    assert_eq!(result["costUsd"], 0.006);
    assert_eq!(result["transparent"], true);
    assert_eq!(room.spending_summary().unwrap().day_usd, 0.006);
    assert!(result["seconds"].as_f64().unwrap() >= 0.0);
    assert_eq!(
        std::fs::read(result["path"].as_str().unwrap()).unwrap(),
        png()
    );
    let attachments = attachments(&room);
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["kind"], "image");
    assert_eq!(attachments[0]["mimeType"], "image/png");
    assert_eq!(
        std::fs::read(attachments[0]["path"].as_str().unwrap()).unwrap(),
        png()
    );
    let error = tools
        .call("generate_image", &json!({ "prompt": "another lighthouse" }))
        .await
        .unwrap_err();
    assert!(
        error.to_lowercase().contains("spend") || error.to_lowercase().contains("budget"),
        "{error}"
    );
    assert_eq!(fake.requests.lock().unwrap().len(), 1);
    assert_eq!(fake.requests.lock().unwrap()[0].aspect, Aspect::Wide);
}

#[tokio::test]
async fn fallback_reserves_again_and_counts_the_failed_attempt_conservatively() {
    let (_dir, room, tools) = room();
    let mut primary = Fake::new("first");
    Arc::get_mut(&mut primary).unwrap().error = Some(ImageError::Refused {
        provider_id: "fake".into(),
        status: 503,
    });
    let fallback = Fake::new("second");
    install(&room, primary.clone(), Some(fallback.clone()));
    settings(&room, 0.015);
    assert!(
        tools
            .call("generate_image", &json!({ "prompt": "a lighthouse" }))
            .await
            .is_err()
    );
    assert_eq!(primary.requests.lock().unwrap().len(), 1);
    assert!(fallback.requests.lock().unwrap().is_empty());
    settings(&room, 0.04);
    let result: Value = serde_json::from_str(
        &tools
            .call("generate_image", &json!({ "prompt": "a lighthouse" }))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["model"], "second");
    assert_eq!(result["costUsd"], 0.016);
    assert_eq!(fallback.requests.lock().unwrap().len(), 1);
    assert_eq!(attachments(&room).len(), 1);
}

#[tokio::test]
async fn client_refusals_release_the_reservation_before_a_paid_fallback() {
    let (_dir, room, tools) = room();
    let mut primary = Fake::new("refused");
    Arc::get_mut(&mut primary).unwrap().error = Some(ImageError::Refused {
        provider_id: "fake".into(),
        status: 400,
    });
    let fallback = Fake::new("second");
    install(&room, primary.clone(), Some(fallback.clone()));
    settings(&room, 0.01);
    let result: Value = serde_json::from_str(
        &tools
            .call("generate_image", &json!({"prompt":"a lighthouse"}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["model"], "second");
    assert_eq!(result["costUsd"], 0.006);
    assert_eq!(room.spending_summary().unwrap().day_usd, 0.006);
    assert_eq!(primary.requests.lock().unwrap().len(), 1);
    assert_eq!(fallback.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn only_client_refusals_release_reservations_and_uncertain_errors_keep_them() {
    let mut errors: Vec<_> = [399, 400, 401, 403, 429, 499, 500, 503]
        .into_iter()
        .map(|status| {
            (
                ImageError::Refused {
                    provider_id: "fake".into(),
                    status,
                },
                if (400..=499).contains(&status) {
                    0.0
                } else {
                    0.01
                },
            )
        })
        .collect();
    errors.extend([
        (
            ImageError::Unreachable {
                provider_id: "fake".into(),
            },
            0.01,
        ),
        (
            ImageError::Malformed {
                provider_id: "fake".into(),
            },
            0.01,
        ),
    ]);
    for (error, expected) in errors {
        let (_dir, room, tools) = room();
        let mut primary = Fake::new("failed");
        Arc::get_mut(&mut primary).unwrap().error = Some(error);
        install(&room, primary, None);
        assert!(
            tools
                .call("generate_image", &json!({"prompt":"draw"}))
                .await
                .is_err()
        );
        let summary = room.spending_summary().unwrap();
        assert_eq!(summary.day_usd, expected);
        assert_eq!(summary.month_usd, expected);
        assert!(attachments(&room).is_empty());
    }
}

#[tokio::test]
async fn missing_reported_cost_is_charged_at_the_reserved_estimate() {
    let (_dir, room, tools) = room();
    let mut fake = Fake::new("estimate");
    Arc::get_mut(&mut fake).unwrap().cost = None;
    install(&room, fake.clone(), None);
    settings(&room, 0.019);
    let result: Value = serde_json::from_str(
        &tools
            .call("generate_image", &json!({ "prompt": "a lighthouse" }))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["costUsd"], 0.01);
    assert!(
        tools
            .call("generate_image", &json!({ "prompt": "another" }))
            .await
            .is_err()
    );
    assert_eq!(fake.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn generated_svg_keeps_the_same_bounded_vector_file_in_workspace_and_chat() {
    let (dir, room, tools) = room();
    let mut fake = Fake::new("svg");
    Arc::get_mut(&mut fake).unwrap().bytes = Some(b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 4096 2048'><circle cx='20' cy='20' r='10'/></svg>".to_vec());
    install(&room, fake, None);
    let result: Value = serde_json::from_str(
        &tools
            .call(
                "generate_image",
                &json!({"prompt":"a diagram", "name":"diagram.png"}),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result["path"],
        json!(real(&dir).join("workspace/diagram.svg"))
    );
    let attachment = &attachments(&room)[0];
    assert_eq!(attachment["mimeType"], "image/svg+xml");
    assert_eq!(attachment["width"], 2048);
    assert_eq!(attachment["height"], 1024);
    assert_eq!(
        std::fs::read(result["path"].as_str().unwrap()).unwrap(),
        std::fs::read(attachment["path"].as_str().unwrap()).unwrap()
    );
}

#[tokio::test]
async fn references_obey_reach_and_valid_references_reach_the_image_model() {
    let (dir, room, tools) = room();
    let fake = Fake::new("edit");
    install(&room, fake.clone(), None);
    std::fs::write(dir.path().join("outside.png"), png()).unwrap();
    for path in [
        "../outside.png".to_string(),
        dir.path()
            .join("outside.png")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert!(
            tools
                .call(
                    "generate_image",
                    &json!({ "prompt": "make this blue", "references": [path] })
                )
                .await
                .is_err()
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            dir.path().join("outside.png"),
            dir.path().join("workspace/escape.png"),
        )
        .unwrap();
        assert!(
            tools
                .call(
                    "generate_image",
                    &json!({ "prompt": "make this blue", "references": ["escape.png"] })
                )
                .await
                .is_err()
        );
    }
    assert!(fake.requests.lock().unwrap().is_empty());
    let reference = dir.path().join("workspace/logo.png");
    std::fs::write(&reference, png()).unwrap();
    tools.call("generate_image", &json!({ "prompt": "make this blue", "references": [reference], "style": "avatar", "aspect": "16:9" })).await.unwrap();
    let requests = fake.requests.lock().unwrap();
    assert_eq!(
        requests[0].references,
        vec![Reference {
            mime: "image/png".into(),
            bytes: png()
        }]
    );
    assert_eq!(requests[0].aspect, Aspect::Square);
    assert!(requests[0].prompt.contains("make this blue"));
}

/// macOS spells its temp folder two ways (`/var` and `/private/var`); a
/// symlink to the same folder stands in for that here.
#[cfg(unix)]
#[tokio::test]
async fn a_reference_named_through_another_spelling_of_the_workspace_is_inside_it() {
    let (dir, room, tools) = room();
    let fake = Fake::new("fake");
    install(&room, fake.clone(), None);
    std::fs::write(dir.path().join("workspace/logo.png"), png()).unwrap();
    let alias = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(dir.path(), alias.path().join("same")).unwrap();
    let reference = alias.path().join("same/workspace/logo.png");
    tools
        .call(
            "generate_image",
            &json!({ "prompt": "make this blue", "references": [reference] }),
        )
        .await
        .unwrap();
    assert_eq!(fake.requests.lock().unwrap()[0].references.len(), 1);
}

#[tokio::test]
async fn disabled_spending_bad_arguments_and_subagents_never_call_a_provider() {
    let (_dir, room, tools) = room();
    let fake = Fake::new("fake");
    install(&room, fake.clone(), None);
    for args in [
        json!({"prompt":""}),
        json!({"prompt":"draw","aspect":"2:1"}),
        json!({"prompt":"draw","style":"missing"}),
        json!({"prompt":"draw","name":"../escape.png"}),
    ] {
        assert!(tools.call("generate_image", &args).await.is_err());
    }
    let run = tools.clone().for_run();
    assert!(
        run.call("generate_image", &json!({"prompt":"draw"}))
            .await
            .unwrap_err()
            .contains("subagent")
    );
    assert!(
        !run.as_dynamic()
            .iter()
            .any(|tool| tool.name() == "generate_image")
    );
    settings(&room, 0.0);
    assert!(
        tools
            .call("generate_image", &json!({"prompt":"draw"}))
            .await
            .is_err()
    );
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn generation_holds_no_room_lock_and_revocation_stops_fallback_and_publication() {
    let (_dir, room, tools) = room();
    let epoch = CapabilityEpoch::default();
    let tools = tools.with_capability(epoch.lease());
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut primary = Fake::new("slow");
    let primary_mut = Arc::get_mut(&mut primary).unwrap();
    primary_mut.entered = Some(entered.clone());
    primary_mut.release = Some(release.clone());
    primary_mut.error = Some(ImageError::Refused {
        provider_id: "fake".into(),
        status: 503,
    });
    let fallback = Fake::new("fallback");
    install(&room, primary, Some(fallback.clone()));
    let generation = tokio::spawn(async move {
        tools
            .call("generate_image", &json!({"prompt":"a lighthouse"}))
            .await
    });
    entered.notified().await;
    assert_eq!(room.log().load(&StreamId::Room)[0]["id"], "ada");
    settings(&room, 0.0);
    epoch.invalidate();
    release.notify_one();
    assert!(generation.await.unwrap().is_err());
    assert!(fallback.requests.lock().unwrap().is_empty());
    assert!(attachments(&room).is_empty());
}

#[tokio::test]
async fn no_image_provider_is_refused_with_a_connection_sentence() {
    let (_dir, _room, tools) = room();
    let error = tools
        .call("generate_image", &json!({"prompt":"a lighthouse"}))
        .await
        .unwrap_err();
    assert!(error.contains("Connect"), "{error}");
}

#[tokio::test]
async fn a_connected_custom_provider_generates_through_the_real_resolver_and_http_adapter() {
    use axum::{Router, body::Bytes, http::Uri};
    use base64::{Engine, prelude::BASE64_STANDARD};
    let (dir, old_room, _tools) = room();
    let log = old_room.log().clone();
    drop(old_room);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let answer =
        json!({"data": [{"b64_json": BASE64_STANDARD.encode(png())}], "usage": {"cost": 0.006}})
            .to_string();
    let app = Router::new().fallback(move |uri: Uri, body: Bytes| {
        seen.lock()
            .unwrap()
            .push((uri.path().to_string(), body.to_vec()));
        let answer = answer.clone();
        async move { answer }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let vault = Arc::new(crate::vault::Vault::open(log.root(), log.clone()).unwrap());
    let credential = vault
        .save_custom(
            None,
            crate::contract::CustomProviderDraft {
                name: "Image harness".into(),
                base_url: url,
                api: crate::contract::OpenAiApi::ChatCompletions,
                models: vec!["fake-image-model".into()],
                secret: None,
            },
        )
        .unwrap();
    let room = Room::new_with_mcp(log, Arc::new(NoKeys), vault);
    let tools = TeammateTools::new(&room, "ada");
    let result: Value = serde_json::from_str(&tools.call("generate_image", &json!({
        "prompt": "a lighthouse", "aspect": "16:9", "transparent": true, "name": "resolved"
    })).await.unwrap()).unwrap();
    assert_eq!(
        result["path"],
        json!(real(&dir).join("workspace/resolved.png"))
    );
    assert_eq!(result["model"], "fake-image-model");
    assert_eq!(result["transparent"], false);
    assert_eq!(result["costUsd"], 0.10);
    assert_eq!(room.spending_summary().unwrap().day_usd, 0.10);
    assert_eq!(attachments(&room).len(), 1);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "/images/generations");
    let sent: Value = serde_json::from_slice(&requests[0].1).unwrap();
    assert_eq!(sent["size"], "1536x1024");
    assert_eq!(sent["model"], "fake-image-model");
    assert!(sent.get("background").is_none());
    assert!(credential.provider_id.starts_with("custom-"));
    server.abort();
}

#[tokio::test]
async fn http_400_refusals_leave_the_ledger_unchanged() {
    use axum::{Router, http::StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_dir, old_room, _tools) = room();
    let log = old_room.log().clone();
    drop(old_room);
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let app = Router::new().fallback(move || {
        seen.fetch_add(1, Ordering::SeqCst);
        async { (StatusCode::BAD_REQUEST, "provider-private-detail") }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let vault = Arc::new(crate::vault::Vault::open(log.root(), log.clone()).unwrap());
    vault
        .save_custom(
            None,
            crate::contract::CustomProviderDraft {
                name: "Refusal harness".into(),
                base_url: url,
                api: crate::contract::OpenAiApi::ChatCompletions,
                models: vec!["fake-image-model".into()],
                secret: None,
            },
        )
        .unwrap();
    let room = Room::new_with_mcp(log, Arc::new(NoKeys), vault);
    let tools = TeammateTools::new(&room, "ada");
    let before = room.spending_summary().unwrap();
    for _ in 0..3 {
        let error = tools
            .call("generate_image", &json!({"prompt":"draw"}))
            .await
            .unwrap_err();
        assert!(!error.contains("provider-private-detail"));
        assert_eq!(room.spending_summary().unwrap(), before);
    }
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    assert!(attachments(&room).is_empty());
    server.abort();
}

#[tokio::test]
async fn corrupt_saved_caps_block_spending_and_generated_files_never_overwrite() {
    let (dir, room, tools) = room();
    let fake = Fake::new("fake");
    install(&room, fake.clone(), None);
    room.log()
        .append(
            &StreamId::Room,
            &json!({"kind":"setting", "id":"spending", "value":{"dayUsd":-1}}),
        )
        .unwrap();
    assert!(
        tools
            .call("generate_image", &json!({"prompt":"draw"}))
            .await
            .is_err()
    );
    assert!(fake.requests.lock().unwrap().is_empty());
    settings(&room, 2.0);
    let path = dir.path().join("workspace/kept.png");
    std::fs::write(&path, b"existing work").unwrap();
    assert!(
        tools
            .call(
                "generate_image",
                &json!({"prompt":"draw", "name":"kept.png"})
            )
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), b"existing work");
    assert!(attachments(&room).is_empty());
}

#[tokio::test]
async fn invalid_saved_image_selections_refuse_before_calling_a_provider() {
    let (_dir, room, tools) = room();
    let fake = Fake::new("fake");
    install(&room, fake.clone(), None);
    for value in [json!({"model":" "}), json!({"provider":""}), json!(null)] {
        room.log()
            .append(
                &StreamId::Room,
                &json!({"kind":"setting", "id":"images", "value":value}),
            )
            .unwrap();
        assert!(
            tools
                .call("generate_image", &json!({"prompt":"draw"}))
                .await
                .is_err()
        );
    }
    assert!(fake.requests.lock().unwrap().is_empty());
    assert!(!room.log().root().join("spending.json").exists());
}

#[tokio::test]
async fn model_reference_limits_are_checked_before_any_spending_reservation() {
    let (dir, room, tools) = room();
    let mut primary = Fake::new("no-references");
    Arc::get_mut(&mut primary).unwrap().max_references = 0;
    let fallback = Fake::new("edits");
    install(&room, primary.clone(), Some(fallback.clone()));
    std::fs::write(dir.path().join("workspace/logo.png"), png()).unwrap();
    let result: Value = serde_json::from_str(
        &tools
            .call(
                "generate_image",
                &json!({
                    "prompt":"make this blue", "references":["logo.png"]
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["costUsd"], 0.006);
    assert!(primary.requests.lock().unwrap().is_empty());
    assert_eq!(fallback.requests.lock().unwrap().len(), 1);
    assert_eq!(room.spending_summary().unwrap().day_usd, 0.006);
}

#[tokio::test]
async fn unreadable_json_cannot_turn_a_saved_zero_cap_back_into_paid_defaults() {
    let (_dir, room, tools) = room();
    let fake = Fake::new("fake");
    install(&room, fake.clone(), None);
    settings(&room, 0.0);
    let path = crate::paths::room_path(room.log().root());
    let original = std::fs::read_to_string(&path).unwrap();
    assert!(original.contains("\"dayUsd\":0.0"));
    let corrupt = original.replace("\"dayUsd\":0.0", "\"dayUsd\":broken");
    std::fs::write(path, corrupt).unwrap();
    let error = tools
        .call("generate_image", &json!({"prompt":"draw"}))
        .await
        .unwrap_err();
    assert!(
        error.contains("settings") || error.contains("stream"),
        "{error}"
    );
    assert!(fake.requests.lock().unwrap().is_empty());
    assert!(!room.log().root().join("spending.json").exists());
}

#[tokio::test]
async fn acp_mcp_transport_lists_and_executes_the_same_image_tool() {
    use rmcp::ServiceExt;
    use rmcp::model::{CallToolRequestParams, ClientInfo};
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    let (_dir, room, tools) = room();
    install(&room, Fake::new("mcp-image"), None);
    let served = crate::mcp::server::serve(tools).await.unwrap();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(served.url()).auth_header(served.token()),
    );
    let client = ClientInfo::default().serve(transport).await.unwrap();
    let listing = client.list_tools(None).await.unwrap();
    assert!(
        listing
            .tools
            .iter()
            .any(|tool| tool.name == "generate_image")
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("generate_image").with_arguments(
                json!({"prompt":"a lighthouse"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert_eq!(attachments(&room).len(), 1);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn setup_draws_a_picture_from_the_name_and_goal_and_charges_it() {
    let (_dir, room, _tools) = room();
    let mut fake = Fake::new("avatar-image");
    let opaque = image::RgbaImage::from_pixel(96, 64, image::Rgba([200, 120, 40, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(opaque)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    Arc::get_mut(&mut fake).unwrap().bytes = Some(bytes.into_inner());
    install(&room, fake.clone(), None);

    let avatar = room.generate_avatar("ada").await.unwrap();
    assert_eq!(avatar.by, crate::contract::AvatarBy::Own);
    assert_eq!(
        room.persona("ada").unwrap().avatar.unwrap().hash,
        avatar.hash
    );
    {
        let requests = fake.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].prompt.contains("a teammate called Ada"));
        assert_eq!(requests[0].aspect, Aspect::Square);
    }
    assert_eq!(room.spending_summary().unwrap().day_usd, 0.006);

    // A picture the person chose is theirs: setup never draws over it.
    let mut persona = room.persona("ada").unwrap();
    persona.avatar.as_mut().unwrap().by = crate::contract::AvatarBy::Person;
    crate::room::append_persona(room.log(), &persona).unwrap();
    let error = room.generate_avatar("ada").await.unwrap_err();
    assert!(error.contains("you chose"), "{error}");
    assert_eq!(fake.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn setup_is_told_at_once_when_no_provider_can_draw() {
    let (_dir, room, _tools) = room();
    let error = room.can_draw_avatar("ada").unwrap_err();
    assert!(error.contains("Connect"), "{error}");
    install(&room, Fake::new("avatar-image"), None);
    room.can_draw_avatar("ada").unwrap();
}

/// The roster shows a picture on its way, and stops showing it however the
/// drawing ends: each change nudges the roster through the teammate's info.
#[tokio::test]
async fn a_picture_being_drawn_is_on_the_roster_until_it_ends() {
    let (_dir, room, _tools) = room();
    let mut infos = room.subscribe_info();
    assert!(!room.drawing("ada"));
    {
        let _drawing = room.start_drawing("ada");
        assert!(room.drawing("ada"));
        assert_eq!(infos.recv().await.unwrap().persona_id, "ada");
    }
    assert!(!room.drawing("ada"));
    assert_eq!(infos.recv().await.unwrap().persona_id, "ada");

    // A refused drawing ends the same way.
    assert!(room.generate_avatar("ada").await.is_err());
    assert!(!room.drawing("ada"));
}
