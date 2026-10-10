use super::*;
use crate::contract::{McpPolicy, Persona, PolicyMode};
use crate::log::Log;
use crate::session::ProviderKeys;
use std::collections::HashMap;
use std::sync::Arc;

struct NoKeys;

impl ProviderKeys for NoKeys {
    fn provider_auth(&self) -> HashMap<String, crate::session::ProviderAuth> {
        HashMap::new()
    }
}

fn room() -> (tempfile::TempDir, Arc<Room>) {
    let dir = tempfile::tempdir().unwrap();
    let log = Log::open(dir.path().join("data"));
    let persona = Persona {
        node: None,
        id: "ada".into(),
        name: "Ada".into(),
        goal: String::new(),
        avatar: None,
        look: None,
        team: None,
        backend_id: "hotline".into(),
        cwd: dir.path().to_string_lossy().into_owned(),
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
        folders: None,
        computer: None,
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1,
        updated_at: 1,
    };
    crate::room::append_persona(&log, &persona).unwrap();
    (dir, Room::new(log, Arc::new(NoKeys)))
}

fn look(room: &Room) -> Option<Look> {
    room.persona("ada").unwrap().look
}

fn clips_kept(dir: &tempfile::TempDir) -> usize {
    std::fs::read_dir(dir.path().join("data/avatars/ada"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "mp4")
                })
                .count()
        })
        .unwrap_or(0)
}

fn encoded(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[tokio::test]
async fn a_style_is_set_and_keeps_the_clips() {
    let (_dir, room) = room();
    room.set_clip("ada", Mood::Sleep, "video/mp4", &encoded(b"zzz"))
        .await
        .unwrap();
    room.set_look("ada", LookStyle::Photo).await.unwrap();
    let look = look(&room).unwrap();
    assert_eq!(look.style, LookStyle::Photo);
    assert!(look.clips.contains_key(&Mood::Sleep));
}

#[tokio::test]
async fn a_clip_is_kept_by_hash_read_back_and_removed_when_no_mood_uses_it() {
    let (dir, room) = room();
    room.set_clip("ada", Mood::Done, "video/mp4", &encoded(b"bounce"))
        .await
        .unwrap();
    let hash = look(&room).unwrap().clips[&Mood::Done].hash.clone();
    let chunk = read(room.log().root(), "ada", &hash, 0).unwrap();
    assert_eq!(chunk.size, 6);

    // Replacing the mood leaves one clip on disk, not two.
    room.set_clip("ada", Mood::Done, "video/mp4", &encoded(b"bounce again"))
        .await
        .unwrap();
    assert_eq!(clips_kept(&dir), 1);

    room.clear_clip("ada", Mood::Done).await.unwrap();
    assert!(look(&room).unwrap().clips.is_empty());
    assert_eq!(clips_kept(&dir), 0);
}

#[tokio::test]
async fn a_clip_must_be_a_video_the_desk_keeps() {
    let (_dir, room) = room();
    for mime_type in ["image/gif", "image/svg+xml"] {
        let error = room
            .set_clip("ada", Mood::Idle, mime_type, &encoded(b"<svg/>"))
            .await
            .unwrap_err();
        assert!(error.contains("MP4 or a WebM"), "{error}");
    }
    let too_big = vec![0u8; MAX_CLIP_BYTES + 1];
    assert!(
        room.set_clip("ada", Mood::Idle, "video/mp4", &encoded(&too_big))
            .await
            .is_err()
    );
    assert_eq!(look(&room), None);
}

#[tokio::test]
async fn a_new_picture_leaves_the_clips_alone() {
    let (dir, room) = room();
    room.set_clip("ada", Mood::Idle, "video/mp4", &encoded(b"idle"))
        .await
        .unwrap();
    crate::session::avatar::remove_except(room.log().root(), "ada", None);
    assert_eq!(clips_kept(&dir), 1);
}

const DRAWING: &str =
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><g data-part="head"/></svg>"#;

fn drawings_kept(dir: &tempfile::TempDir) -> usize {
    std::fs::read_dir(dir.path().join("data/avatars/ada"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "svg")
                })
                .count()
        })
        .unwrap_or(0)
}

#[tokio::test]
async fn a_drawing_is_kept_by_hash_beside_the_clips_and_removed_when_cleared() {
    let (dir, room) = room();
    room.set_clip("ada", Mood::Idle, "video/mp4", &encoded(b"idle"))
        .await
        .unwrap();
    room.set_drawing("ada", DRAWING).await.unwrap();
    let drawing = look(&room).unwrap().drawing.unwrap();
    assert_eq!(drawing.mime_type, "image/svg+xml");
    let chunk = read(room.log().root(), "ada", &drawing.hash, 0).unwrap();
    assert_eq!(chunk.size, DRAWING.len() as i64);
    assert_eq!(clips_kept(&dir), 1);

    // A new style keeps the drawing, and a new drawing replaces the old one on disk.
    room.set_look("ada", LookStyle::Drawn).await.unwrap();
    assert!(look(&room).unwrap().drawing.is_some());
    room.set_drawing("ada", &DRAWING.replace("head", "jaw"))
        .await
        .unwrap();
    assert_eq!(drawings_kept(&dir), 1);

    room.clear_drawing("ada").await.unwrap();
    assert_eq!(look(&room).unwrap().drawing, None);
    assert_eq!(drawings_kept(&dir), 0);
    assert_eq!(clips_kept(&dir), 1);
}

#[tokio::test]
async fn a_drawing_must_be_an_svg_without_scripts_entities_or_pages() {
    let (_dir, room) = room();
    for (svg, said) in [
        ("<png/>", "is an SVG"),
        (r#"<!DOCTYPE svg [<!ENTITY a "a">]><svg/>"#, "doctype"),
        ("<svg><script>alert(1)</script></svg>", "scripts"),
        (
            "<svg><foreignObject><iframe/></foreignObject></svg>",
            "scripts",
        ),
    ] {
        let error = room.set_drawing("ada", svg).await.unwrap_err();
        assert!(error.contains(said), "{svg}: {error}");
    }
    let too_big = format!("<svg>{}</svg>", " ".repeat(MAX_DRAWING_BYTES));
    assert!(room.set_drawing("ada", &too_big).await.is_err());
    assert_eq!(look(&room), None);
}
