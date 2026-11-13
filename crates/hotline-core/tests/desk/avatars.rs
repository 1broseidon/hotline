use super::*;

fn png_edge(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "a kept picture is a PNG");
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (width, height)
}

/// With no image provider on the desk, setup's offer still gives the
/// teammate a picture, drawn here and its own; the person can then pick one
/// of the pictures the desk offers, which is kept as theirs, and setup's
/// offer never draws over it. All of it over the wire the window uses.
#[tokio::test(flavor = "multi_thread")]
async fn a_picture_is_drawn_offered_and_picked_without_an_image_provider() {
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

    // Setup's offer: no provider, so the default style, as the teammate's own.
    let started = client
        .call("avatar.generate", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let own = client
        .next_where(Duration::from_secs(30), |frame| {
            is_sub(frame, roster, "event") && frame["event"]["persona"]["avatar"].is_object()
        })
        .await["event"]["persona"]["avatar"]
        .clone();
    assert_eq!(own["by"], "self");

    // The pictures on offer: the same for the same round, others for the next.
    let offered = client
        .call(
            "avatar.candidates",
            json!({ "personaId": persona_id, "round": 2 }),
        )
        .await["result"]
        .clone();
    let candidates = offered.as_array().unwrap();
    assert!(candidates.len() >= 8, "{offered}");
    for candidate in candidates {
        assert!(candidate["svg"].as_str().unwrap().starts_with("<svg"));
    }
    let again = client
        .call(
            "avatar.candidates",
            json!({ "personaId": persona_id, "round": 2 }),
        )
        .await;
    assert_eq!(again["result"], offered);
    let next = client
        .call(
            "avatar.candidates",
            json!({ "personaId": persona_id, "round": 3 }),
        )
        .await;
    assert_ne!(next["result"], offered);

    // A style the desk does not draw is refused; one it offered is kept.
    let refused = client
        .call(
            "avatar.pick",
            json!({ "personaId": persona_id, "style": "bottts", "seed": "x" }),
        )
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    let choice = &candidates[1];
    let picked = client
        .call(
            "avatar.pick",
            json!({ "personaId": persona_id, "style": choice["style"], "seed": choice["seed"] }),
        )
        .await;
    assert_eq!(picked["ok"], true, "{picked}");
    assert_eq!(picked["result"]["by"], "person");
    assert_ne!(picked["result"]["hash"], own["hash"]);
    let hash = picked["result"]["hash"].as_str().unwrap().to_string();
    let kept = std::fs::read(root.join(format!("avatars/{persona_id}/{hash}.png"))).unwrap();
    assert_eq!(png_edge(&kept), (512, 512));
    let read = client
        .call(
            "avatar.read",
            json!({ "personaId": persona_id, "hash": hash }),
        )
        .await;
    assert_eq!(read["result"]["mimeType"], "image/png", "{read}");

    // Setup's offer leaves the person's choice alone.
    let refused = client
        .call("avatar.generate", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains("you chose"),
        "{refused}"
    );
}
