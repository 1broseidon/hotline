use super::*;

/// Setup's Picture: `avatar.generate` gives a new teammate a face at once,
/// drawn on the desk with no image provider and kept as its own with the
/// recipe it was seeded from, and makes no job: no model turn is spent on a
/// face until the person asks the teammate for one. All of it over the wire
/// the window uses.
#[tokio::test(flavor = "multi_thread")]
async fn setup_gives_a_seeded_face_and_no_job() {
    let (root, port) = open("avatars").await;
    let mut client = Client::connect(port, TOKEN).await;
    let roster = client.subscribe(json!({ "view": "roster" })).await;
    client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "snapshot")
        })
        .await;
    let created = client
        .call("persona.create", json!({ "draft": { "name": "Ada" } }))
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let images = client.call("images.status", json!({})).await;
    assert_eq!(images["result"]["available"], false, "{images}");

    let started = client
        .call("avatar.generate", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let face = client
        .next_where(Duration::from_secs(30), |frame| {
            is_sub(frame, roster, "event") && frame["event"]["persona"]["avatar"].is_object()
        })
        .await["event"]["persona"]["avatar"]
        .clone();
    assert_eq!(face["by"], "self");
    assert_eq!(face["recipe"]["madeBy"], "desk");
    assert_eq!(face["recipe"]["seed"], persona_id.as_str());
    assert!(face["recipe"]["parts"].is_object(), "{face}");
    let hash = face["hash"].as_str().unwrap();
    let kept = std::fs::read(root.join(format!("avatars/{persona_id}/{hash}.png"))).unwrap();
    assert_eq!(&kept[..8], b"\x89PNG\r\n\x1a\n");
    let read = client
        .call(
            "avatar.read",
            json!({ "personaId": persona_id, "hash": hash, "size": 64 }),
        )
        .await;
    assert_eq!(read["result"]["mimeType"], "image/png", "{read}");
    // Every seeded style blinks, so the face comes with its eyes shut, read
    // the same way at a phone's size; pixelbot's one-row `soft` eyes are the
    // only ones a blink leaves as they are.
    if face["recipe"]["parts"]["eyes"] != "soft" {
        let shut = face["poses"]["shut"]
            .as_str()
            .expect("a seeded face blinks");
        let read = client
            .call(
                "avatar.read",
                json!({ "personaId": persona_id, "hash": shut, "size": 64 }),
            )
            .await;
        assert_eq!(read["result"]["mimeType"], "image/png", "{read}");
    }

    let jobs = client.call("schedule.list", json!({})).await["result"].clone();
    assert_eq!(jobs, json!([]), "{jobs}");
}

/// A face kept before there were poses, on a desk opened by this release,
/// is on the roster with its eyes-shut picture, which any seat reads by
/// hash: the desk gives it when it opens, before the window connects.
#[tokio::test(flavor = "multi_thread")]
async fn a_face_kept_before_poses_has_one_once_the_desk_opens() {
    let root = scratch("avatars-poses");
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(4, 4, image::Rgb([0, 43, 45]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let picture = png.into_inner();
    let hash = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&picture));
    std::fs::create_dir_all(root.join("avatars/ada")).unwrap();
    std::fs::write(root.join(format!("avatars/ada/{hash}.png")), &picture).unwrap();
    let log = hotline_core::log::Log::open(&root);
    log.append(
        &hotline_core::log::StreamId::Room,
        &json!({
            "kind": "persona", "id": "ada", "name": "Ada", "goal": "", "backendId": "hotline",
            "cwd": root, "mcpPolicy": { "mode": "none", "serverIds": [] },
            "sessionCheckpoints": [], "createdAt": 1, "updatedAt": 1,
            "avatar": {
                "hash": hash, "by": "self", "updatedAt": "2026-11-01T00:00:00.000Z",
                "recipe": {
                    "style": "moods", "parts": {}, "seed": "ada", "madeBy": "teammate",
                    "colors": { "face": "#48b8bc", "background": "#002b2d" }
                }
            }
        }),
    )
    .unwrap();
    drop(log);

    let mut client = Client::connect(open_at(&root), TOKEN).await;
    let roster = client.subscribe(json!({ "view": "roster" })).await;
    let snapshot = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "snapshot")
        })
        .await;
    let avatar = snapshot["snapshot"][0]["persona"]["avatar"].clone();
    assert_eq!(avatar["hash"], hash.as_str(), "{avatar}");
    let shut = avatar["poses"]["shut"]
        .as_str()
        .expect("a moods face blinks");
    let read = client
        .call(
            "avatar.read",
            json!({ "personaId": "ada", "hash": shut, "offset": 0 }),
        )
        .await;
    assert_eq!(read["result"]["mimeType"], "image/png", "{read}");
}
