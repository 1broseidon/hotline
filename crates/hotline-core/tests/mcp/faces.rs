//! `face_options` and `make_face` through the real core, on both drivers,
//! with the scripted desk of `post_to_chat.rs`: a teammate reads the styles
//! and one style's parts, makes its face, changes one part of it, is refused
//! a colour that is not a `#rrggbb`, and switches style. Each face is kept as
//! the teammate's own picture with its recipe, read back off the disk, and the
//! picture itself is handed back to the agent with the words. A face in a
//! style that blinks is kept with its eyes shut beside it, read over the wire
//! by hash, and that pose goes when the face is replaced by one that does not.

use super::post_to_chat::{Desk, Driver, open};
use hotline_core::log::{Log, StreamId};
use serde_json::{Value, json};

/// The teammate's record as the room stream last wrote it.
fn persona_on_disk(desk: &Desk, persona_id: &str) -> Value {
    Log::open(desk.root.path())
        .load(&StreamId::Room)
        .into_iter()
        .rev()
        .find(|event| event["kind"] == "persona" && event["id"] == persona_id)
        .expect("the teammate is on the room stream")
}

/// Some line of what the calls answered says `sentence`. Hotline Agent's
/// scripted model counts a picture as an answer of its own, so the lines are
/// not one per call there.
fn said(results: &[String], sentence: &str) {
    assert!(
        results.iter().any(|line| line.contains(sentence)),
        "a call should say {sentence:?}: {results:#?}"
    );
}

async fn a_teammate_makes_and_changes_its_own_face(driver: Driver) {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", driver, false).await;
    let script = [
        "CALL face_options {}",
        r#"CALL face_options {"style":"sprouts"}"#,
        r##"CALL make_face {"style":"sprouts","parts":{"eyes":"wide"},"colors":{"pot":"#48b8bc"}}"##,
        r#"CALL make_face {"style":"sprouts","parts":{"mouth":"grin"}}"#,
        r#"CALL make_face {"style":"sprouts","colors":{"pot":"teal"}}"#,
    ]
    .join("\n");
    let sent = desk
        .client
        .call("session.prompt", json!({"personaId": ada, "text": script}))
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let results = desk.results(tape).await;
    for sentence in [
        r#""bubble":{"name""#,
        "wide",
        r#""madeBy":"teammate""#,
        r#""mouth":"grin""#,
        "must be a #rrggbb hex colour",
    ] {
        said(&results, sentence);
    }
    // The picture goes back to the agent with the words: an ACP child is
    // handed it as an image block; Hotline Agent's request carries it, which
    // this scripted model, reading no images, is shown as a placeholder.
    match driver {
        Driver::Acp => {
            let pictured = results
                .iter()
                .filter(|line| line.starts_with("make_face ok with 1 picture"))
                .count();
            assert_eq!(pictured, 2, "{results:#?}");
            assert!(!results[0].contains("picture:"), "{results:#?}");
        }
        Driver::HotlineAgent => said(&results, "[image image/png"),
    }

    let avatar = persona_on_disk(&desk, &ada)["avatar"].clone();
    assert_eq!(avatar["by"], "self");
    let recipe = &avatar["recipe"];
    assert_eq!(recipe["style"], "sprouts");
    assert_eq!(recipe["madeBy"], "teammate");
    assert_eq!(recipe["parts"]["eyes"], "wide", "the earlier choice stays");
    assert_eq!(recipe["parts"]["mouth"], "grin");
    assert_eq!(
        recipe["colors"]["pot"], "#48b8bc",
        "the refused colour changed nothing"
    );
    let hash = avatar["hash"].as_str().unwrap();
    assert!(
        desk.root
            .path()
            .join(format!("avatars/{ada}/{hash}.png"))
            .is_file()
    );
    // Sprouts blink, so the face is kept with its eyes shut too, and any
    // seat reads that picture by its hash as it reads the face.
    let shut = avatar["poses"]["shut"]
        .as_str()
        .expect("a sprouts face blinks");
    assert_ne!(shut, hash);
    let read = desk
        .client
        .call(
            "avatar.read",
            json!({"personaId": ada, "hash": shut, "offset": 0}),
        )
        .await;
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(read["result"]["mimeType"], "image/png", "{read}");

    let switched = r#"CALL make_face {"style":"pixelbot","parts":{"eyes":"soft"}}"#;
    let sent = desk
        .client
        .call(
            "session.prompt",
            json!({"personaId": ada, "text": switched}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    said(&desk.results(tape).await, r#""style":"pixelbot""#);
    let person = persona_on_disk(&desk, &ada)["avatar"].clone();
    assert!(person.get("poses").is_none(), "one-row eyes do not blink");
    assert!(
        !desk
            .root
            .path()
            .join(format!("avatars/{ada}/{shut}.png"))
            .exists(),
        "the old face's pose went with it"
    );
    let recipe = person["recipe"].clone();
    assert_eq!(recipe["style"], "pixelbot");
    assert!(
        recipe["parts"].get("plant").is_none(),
        "a new style starts afresh"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_makes_and_changes_its_own_face() {
    a_teammate_makes_and_changes_its_own_face(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_child_makes_and_changes_its_own_face() {
    a_teammate_makes_and_changes_its_own_face(Driver::Acp).await;
}
