use super::*;
use crate::contract::{McpPolicy, Persona, PolicyMode};
use crate::log::Log;
use crate::mcp::server::TeammateTools;
use crate::session::ProviderKeys;
use std::collections::HashMap;
use std::sync::Arc;

struct NoKeys;

impl ProviderKeys for NoKeys {
    fn provider_auth(&self) -> HashMap<String, crate::session::ProviderAuth> {
        HashMap::new()
    }
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
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1,
        updated_at: 1,
    };
    crate::room::append_persona(&log, &persona).unwrap();
    let room = Room::new(log, Arc::new(NoKeys));
    let tools = TeammateTools::new(&room, "ada");
    (dir, room, tools)
}

/// A 200 by 100 picture, transparent but for a 40 pixel square in it.
fn margined() -> Vec<u8> {
    let mut image = RgbaImage::new(200, 100);
    for x in 100..140 {
        for y in 30..70 {
            image.put_pixel(x, y, image::Rgba([200, 60, 20, 255]));
        }
    }
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn kept(dir: &tempfile::TempDir) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.path().join("data/avatars/ada"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn avatar(room: &Room) -> Option<Avatar> {
    room.persona("ada").unwrap().avatar
}

#[tokio::test]
async fn a_workspace_picture_is_trimmed_squared_and_kept_under_its_hash() {
    let (dir, room, tools) = room();
    std::fs::write(dir.path().join("workspace/otter.png"), margined()).unwrap();

    let result: Value = serde_json::from_str(
        &tools
            .call("set_avatar", &json!({ "path": "otter.png" }))
            .await
            .unwrap(),
    )
    .unwrap();

    let avatar = avatar(&room).expect("the record carries the picture");
    assert_eq!(result["hash"], avatar.hash);
    assert_eq!(avatar.by, AvatarBy::Own);
    let file = dir
        .path()
        .join(format!("data/avatars/ada/{}.png", avatar.hash));
    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(hex::encode(Sha256::digest(&bytes)), avatar.hash);
    let picture = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(picture.dimensions(), (512, 512));
    // The subject fills the frame but for its padding; the margin is gone.
    assert_eq!(picture.get_pixel(256, 256)[3], 255);
    assert_eq!(picture.get_pixel(40, 256)[3], 255);
    assert_eq!(picture.get_pixel(256, 40)[3], 255);
    assert_eq!(picture.get_pixel(5, 5)[3], 0);
}

#[tokio::test]
async fn a_new_picture_replaces_the_old_and_clearing_removes_it() {
    let (dir, room, tools) = room();
    std::fs::write(dir.path().join("workspace/one.png"), margined()).unwrap();
    let mut other = RgbaImage::from_pixel(64, 64, image::Rgba([10, 20, 30, 255]));
    other.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(other)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.path().join("workspace/two.png"), bytes.into_inner()).unwrap();

    tools
        .call("set_avatar", &json!({ "path": "one.png" }))
        .await
        .unwrap();
    let first = avatar(&room).unwrap().hash;
    tools
        .call("set_avatar", &json!({ "path": "two.png" }))
        .await
        .unwrap();
    let second = avatar(&room).unwrap().hash;
    assert_ne!(first, second);
    assert_eq!(kept(&dir), vec![format!("{second}.png")]);

    tools
        .call("set_avatar", &json!({ "clear": true }))
        .await
        .unwrap();
    assert_eq!(avatar(&room), None);
    assert!(kept(&dir).is_empty());
}

#[tokio::test]
async fn a_picture_the_person_chose_is_left_alone() {
    let (dir, room, tools) = room();
    std::fs::write(dir.path().join("workspace/otter.png"), margined()).unwrap();
    let mut persona = room.persona("ada").unwrap();
    let hash = "a".repeat(64);
    persona.avatar = Some(Avatar {
        hash: hash.clone(),
        by: AvatarBy::Person,
        updated_at: "2026-09-30T00:00:00.000Z".into(),
    });
    crate::room::append_persona(room.log(), &persona).unwrap();

    for arguments in [json!({ "path": "otter.png" }), json!({ "clear": true })] {
        let error = tools.call("set_avatar", &arguments).await.unwrap_err();
        assert!(error.contains("The person chose"), "{error}");
    }
    assert_eq!(avatar(&room).unwrap().hash, hash);
}

#[tokio::test]
async fn only_an_image_inside_the_workspace_can_be_a_picture() {
    let (dir, room, tools) = room();
    std::fs::write(dir.path().join("workspace/notes.txt"), "not a picture").unwrap();
    std::fs::write(dir.path().join("outside.png"), margined()).unwrap();
    std::fs::write(dir.path().join("workspace/empty.png"), {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::new(8, 8))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    })
    .unwrap();

    let outside = dir.path().join("outside.png");
    for (path, wanted) in [
        ("notes.txt", "must be a PNG, JPEG or WebP"),
        ("empty.png", "The picture is empty"),
        ("../outside.png", ""),
        (outside.to_str().unwrap(), "must be inside your workspace"),
        ("missing.png", ""),
    ] {
        let error = tools
            .call("set_avatar", &json!({ "path": path }))
            .await
            .unwrap_err();
        assert!(error.contains(wanted), "{path}: {error}");
    }
    assert_eq!(avatar(&room), None);
    assert!(kept(&dir).is_empty());
}

#[tokio::test]
async fn set_avatar_needs_a_path_or_clear_and_says_what_it_got() {
    let (_dir, _room, tools) = room();
    for (arguments, sent) in [
        (json!({}), "nothing"),
        (json!({ "clear": false }), "`clear`"),
        (json!({ "path": " " }), "`path`"),
        (json!({ "path": 7, "clear": null }), "`path`, `clear`"),
    ] {
        let error = tools.call("set_avatar", &arguments).await.unwrap_err();
        assert!(error.contains("either a `path`"), "{error}");
        assert!(
            error.ends_with(&format!("this call sent {sent}.")),
            "{error}"
        );
    }
}

#[test]
fn a_path_wins_over_whatever_else_the_model_filled_in() {
    for arguments in [
        json!({ "path": " a.png ", "clear": true }),
        json!({ "path": "a.png", "clear": false }),
        json!({ "path": "a.png", "clear": null, "by": "person" }),
    ] {
        assert_eq!(
            requested_path(&arguments).unwrap(),
            Some("a.png".into()),
            "{arguments}"
        );
    }
    assert_eq!(requested_path(&json!({ "clear": true })).unwrap(), None);
    assert_eq!(
        requested_path(&json!({ "path": "", "clear": true })).unwrap(),
        None
    );
}

#[test]
fn a_picture_is_read_only_by_a_genuine_hash() {
    let dir = tempfile::tempdir().unwrap();
    let png = square(&margined()).unwrap();
    let hash = hex::encode(Sha256::digest(&png));
    store(dir.path(), "ada", &hash, &png).unwrap();
    store(dir.path(), "ada", &hash, &png).unwrap();

    let chunk = read(dir.path(), "ada", &hash, 0).unwrap();
    assert_eq!(chunk.mime_type, "image/png");
    assert_eq!(chunk.size as usize, png.len());
    for bad in [
        "",
        "../../room",
        &hash.to_uppercase(),
        &hash[..63],
        &format!("{hash}0"),
    ] {
        assert_eq!(
            read(dir.path(), "ada", bad, 0).unwrap_err(),
            "A picture is named by 64 lowercase hex digits."
        );
    }
    assert_eq!(
        read(dir.path(), "bob", &hash, 0).unwrap_err(),
        "That teammate has no such picture."
    );
}
