use super::*;

/// Setup's Picture: `avatar.generate` gives a new teammate a face at once,
/// drawn on the desk with no image provider and kept as its own with the
/// recipe it was seeded from, and leaves the teammate a quiet one-shot job
/// to make its own face, which posts nothing to the chat. All of it over the
/// wire the window uses.
#[tokio::test(flavor = "multi_thread")]
async fn setup_gives_a_seeded_face_and_a_quiet_job_to_make_its_own() {
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

    let jobs = client.call("schedule.list", json!({})).await["result"].clone();
    let jobs = jobs.as_array().unwrap();
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0]["personaId"], persona_id.as_str());
    assert_eq!(jobs[0]["kind"], "schedule");
    assert_eq!(jobs[0]["quiet"], true);
    assert!(
        jobs[0]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("Make your own face."),
        "{jobs:?}"
    );
}
