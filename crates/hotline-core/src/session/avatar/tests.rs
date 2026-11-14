use super::*;
use crate::contract::SkillPolicy;
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
        bubble: None,
        backend_id: "hotline".into(),
        cwd: workspace.to_string_lossy().into_owned(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        config_choices: std::collections::BTreeMap::new(),
        effort_by_model: std::collections::BTreeMap::new(),
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::None,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: None,
        secrets: None,
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

fn kept_files(dir: &tempfile::TempDir) -> Vec<String> {
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
    assert_eq!(kept_files(&dir), vec![format!("{second}.png")]);

    tools
        .call("set_avatar", &json!({ "clear": true }))
        .await
        .unwrap();
    assert_eq!(avatar(&room), None);
    assert_eq!(kept_files(&dir), [] as [String; 0]);
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
        recipe: None,
        poses: None,
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
    assert_eq!(kept_files(&dir), [] as [String; 0]);
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

fn decoded(content: &crate::mcp::CallContent) -> RgbaImage {
    use base64::Engine as _;
    assert_eq!(content.images.len(), 1);
    assert_eq!(content.images[0].mime_type, "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(&content.images[0].data)
        .unwrap();
    image::load_from_memory_with_format(&png, ImageFormat::Png)
        .unwrap()
        .to_rgba8()
}

/// `face_options` lists the styles, the face now and the bubble colour,
/// and one style's parts and colours, through the real handler.
#[tokio::test]
async fn face_options_lists_the_styles_and_then_a_styles_parts() {
    let (_dir, _room, tools) = room();
    let listed: Value =
        serde_json::from_str(&tools.call("face_options", &json!({})).await.unwrap()).unwrap();
    let styles: Vec<&str> = listed["styles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|style| style["style"].as_str().unwrap())
        .collect();
    assert_eq!(styles, ["pixelbot", "moods", "sprouts"]);
    assert!(
        listed["styles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|style| style["yourColors"]["background"].is_string())
    );
    assert_eq!(listed["bubble"]["name"], "teal");
    assert!(listed.get("tags").is_none());

    let sprouts: Value = serde_json::from_str(
        &tools
            .call("face_options", &json!({ "style": "sprouts" }))
            .await
            .unwrap(),
    )
    .unwrap();
    let eyes = sprouts["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["part"] == "eyes")
        .unwrap();
    assert!(
        eyes["variants"]
            .as_array()
            .unwrap()
            .contains(&json!("wide"))
    );
    assert!(
        sprouts["colors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|slot| slot["slot"] == "pot")
    );

    let error = tools
        .call("face_options", &json!({ "style": "bottts" }))
        .await
        .unwrap_err();
    assert!(error.contains("The styles are"), "{error}");
}

/// `make_face` keeps the face as the teammate's own picture with its recipe
/// and hands the picture back; asked again in the same style it changes
/// only what it is told, and a new style starts afresh.
#[tokio::test]
async fn make_face_keeps_a_face_and_changes_only_what_it_is_told() {
    let (dir, room, tools) = room();
    let first = tools
        .answer(
            "make_face",
            &json!({ "style": "sprouts", "colors": { "pot": "#48b8bc" } }),
        )
        .await
        .unwrap();
    assert_eq!(decoded(&first).dimensions(), (512, 512));
    let kept = avatar(&room).unwrap();
    assert_eq!(kept.by, AvatarBy::Own);
    let recipe = kept.recipe.clone().unwrap();
    assert_eq!(recipe.style, "sprouts");
    assert_eq!(recipe.colors["pot"], "#48b8bc");
    assert_eq!(recipe.seed, "ada");
    assert_eq!(recipe.made_by, crate::contract::FaceMaker::Teammate);
    let said: Value = serde_json::from_str(&first.text).unwrap();
    assert_eq!(said["hash"], kept.hash);
    assert_eq!(said["recipe"], json!(recipe));
    let bytes = std::fs::read(
        dir.path()
            .join(format!("data/avatars/ada/{}.png", kept.hash)),
    )
    .unwrap();
    assert_eq!(hex::encode(Sha256::digest(&bytes)), kept.hash);

    tools
        .answer(
            "make_face",
            &json!({ "style": "sprouts", "parts": { "mouth": "grin" } }),
        )
        .await
        .unwrap();
    let hat = avatar(&room).unwrap().recipe.unwrap();
    assert_eq!(hat.parts["mouth"].as_deref(), Some("grin"));
    for (name, variant) in &hat.parts {
        if name != "mouth" {
            assert_eq!(variant, &recipe.parts[name], "{name}");
        }
    }
    assert_eq!(hat.colors, recipe.colors);
    let now = avatar(&room).unwrap();
    let mut kept = vec![
        format!("{}.png", now.hash),
        format!("{}.png", now.poses.unwrap().shut),
    ];
    kept.sort();
    assert_eq!(kept_files(&dir), kept, "the old face and its pose are gone");

    tools
        .answer("make_face", &json!({ "style": "moods" }))
        .await
        .unwrap();
    let moods = avatar(&room).unwrap().recipe.unwrap();
    assert_eq!(moods.style, "moods");
    assert!(moods.parts.contains_key("face") && !moods.parts.contains_key("plant"));
    assert!(!moods.colors.contains_key("pot"));
}

/// A bad call is refused and changes nothing, and so is any call over a
/// picture the person chose.
#[tokio::test]
async fn make_face_refuses_unknown_names_bad_colours_and_the_persons_choice() {
    let (_dir, room, tools) = room();
    for (arguments, said) in [
        (json!({ "style": "bottts" }), "The styles are"),
        (
            json!({ "style": "sprouts", "parts": { "wings": "big" } }),
            "Its parts are",
        ),
        (
            json!({ "style": "sprouts", "colors": { "pot": "teal" } }),
            "#rrggbb",
        ),
    ] {
        let error = tools.call("make_face", &arguments).await.unwrap_err();
        assert!(error.contains(said), "{arguments}: {error}");
    }
    assert_eq!(avatar(&room), None);

    let mut persona = room.persona("ada").unwrap();
    persona.avatar = Some(Avatar {
        hash: "a".repeat(64),
        by: AvatarBy::Person,
        updated_at: "2026-11-13T00:00:00.000Z".into(),
        recipe: None,
        poses: None,
    });
    crate::room::append_persona(room.log(), &persona).unwrap();
    let error = tools
        .call("make_face", &json!({ "style": "sprouts" }))
        .await
        .unwrap_err();
    assert!(
        error.contains("The person chose your current picture"),
        "{error}"
    );
    assert_eq!(avatar(&room).unwrap().hash, "a".repeat(64));
}

/// Setup gives a face at once, seeded by the desk in the bubble colour,
/// and asks the teammate, through a quiet job, to make its own; it never
/// draws over the person's choice.
#[tokio::test]
async fn setup_seeds_a_face_and_asks_the_teammate_for_its_own() {
    let (_dir, room, _tools) = room();
    let seeded = room.generate_avatar("ada").await.unwrap();
    assert_eq!(seeded.by, AvatarBy::Own);
    let recipe = seeded.recipe.clone().unwrap();
    assert_eq!(recipe.made_by, crate::contract::FaceMaker::Desk);
    assert!(
        recipe.colors.values().any(|color| color == "#48b8bc"),
        "the bubble colour is in the face: {recipe:?}"
    );
    assert_eq!(avatar(&room).unwrap(), seeded);
    let jobs = room.schedule_list();
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].prompt.starts_with("Make your own face."));
    assert_eq!(jobs[0].quiet, Some(true));
    assert_eq!(jobs[0].persona_id, "ada");
    assert!(!room.drawing("ada"));

    let mut persona = room.persona("ada").unwrap();
    persona.avatar.as_mut().unwrap().by = AvatarBy::Person;
    crate::room::append_persona(room.log(), &persona).unwrap();
    let error = room.generate_avatar("ada").await.unwrap_err();
    assert!(error.contains("you chose"), "{error}");
    assert_eq!(room.schedule_list().len(), 1);
}

/// A record from before faces has no recipe and reads as it did; a face
/// made over a picture from a file starts afresh.
#[tokio::test]
async fn a_picture_without_a_recipe_still_loads_and_a_face_replaces_it() {
    let old: Avatar = serde_json::from_value(
        json!({ "hash": "b".repeat(64), "by": "self", "updatedAt": "2026-10-01T00:00:00.000Z" }),
    )
    .unwrap();
    assert_eq!(old.recipe, None);
    assert!(serde_json::to_value(&old).unwrap().get("recipe").is_none());

    let (dir, room, tools) = room();
    std::fs::write(dir.path().join("workspace/otter.png"), margined()).unwrap();
    tools
        .call("set_avatar", &json!({ "path": "otter.png" }))
        .await
        .unwrap();
    assert_eq!(avatar(&room).unwrap().recipe, None);
    tools
        .answer("make_face", &json!({ "style": "sprouts" }))
        .await
        .unwrap();
    assert_eq!(avatar(&room).unwrap().recipe.unwrap().style, "sprouts");
}

/// A subagent run and a voice front have neither tool, and a revoked
/// session's tools draw nothing; a job's thread has both, which is how a new
/// teammate makes its own face.
#[tokio::test]
async fn only_the_teammates_own_threads_make_its_face() {
    let (_dir, room, _tools) = room();
    let face = json!({ "style": "sprouts" });
    for (tools, who) in [
        (TeammateTools::new(&room, "ada").for_run(), "A subagent"),
        (
            TeammateTools::new(&room, "ada").for_front(),
            "A voice front",
        ),
    ] {
        for name in ["make_face", "face_options"] {
            let error = tools.call(name, &face).await.unwrap_err();
            assert!(error.starts_with(who), "{name}: {error}");
        }
    }
    assert_eq!(avatar(&room), None);

    let epoch = crate::driver::CapabilityEpoch::default();
    let lease = epoch.lease();
    let revoked = TeammateTools::new(&room, "ada").with_capability(lease.clone());
    lease.revoke();
    assert!(revoked.call("make_face", &face).await.is_err());
    assert_eq!(avatar(&room), None);

    let job = TeammateTools::new(&room, "ada").for_job("job-1");
    job.call("make_face", &face).await.unwrap();
    assert_eq!(avatar(&room).unwrap().recipe.unwrap().style, "sprouts");
}

/// A teammate whose face is in a style the desk no longer draws keeps its
/// picture: asked for a change in that style it is told the style is gone,
/// and nothing is written; a face in a style there is replaces it.
#[tokio::test]
async fn a_face_in_a_removed_style_stays_until_a_new_one_is_made() {
    let (_dir, room, tools) = room();
    let mut persona = room.persona("ada").unwrap();
    let kept = Avatar {
        hash: "c".repeat(64),
        by: AvatarBy::Own,
        updated_at: "2026-11-13T00:00:00.000Z".into(),
        recipe: Some(crate::contract::AvatarRecipe {
            style: "open-peeps".into(),
            parts: [("head".to_string(), Some("afro".to_string()))].into(),
            colors: [("skin".to_string(), "#d08b5b".to_string())].into(),
            seed: "ada".into(),
            made_by: crate::contract::FaceMaker::Teammate,
        }),
        poses: None,
    };
    persona.avatar = Some(kept.clone());
    crate::room::append_persona(room.log(), &persona).unwrap();

    let error = tools
        .call(
            "make_face",
            &json!({ "style": "open-peeps", "parts": { "head": "bun" } }),
        )
        .await
        .unwrap_err();
    assert!(
        error.starts_with("Your face is in the open-peeps style, which Hotline no longer draws."),
        "{error}"
    );
    assert_eq!(avatar(&room).unwrap(), kept);
    let listed: Value =
        serde_json::from_str(&tools.call("face_options", &json!({})).await.unwrap()).unwrap();
    assert!(listed["currentStyleGone"].is_string(), "{listed}");

    tools
        .answer("make_face", &json!({ "style": "moods" }))
        .await
        .unwrap();
    assert_eq!(avatar(&room).unwrap().recipe.unwrap().style, "moods");
}

/// A face in a style that blinks is kept with its eyes shut beside it, under
/// the hash the record names, read like the picture; a face that does not
/// blink has no pose, and its predecessor's goes with the old picture; and
/// clearing takes the pose too.
#[tokio::test]
async fn a_face_that_blinks_keeps_its_eyes_shut_beside_it() {
    let (dir, room, tools) = room();
    tools
        .answer("make_face", &json!({ "style": "moods" }))
        .await
        .unwrap();
    let moods = avatar(&room).unwrap();
    let shut = moods.poses.clone().expect("a moods face blinks").shut;
    assert_ne!(shut, moods.hash);
    let mut both = vec![format!("{}.png", moods.hash), format!("{shut}.png")];
    both.sort();
    assert_eq!(kept_files(&dir), both);
    let read = super::read(&dir.path().join("data"), "ada", &shut, 0).unwrap();
    assert_eq!(read.mime_type, "image/png");
    let bytes = std::fs::read(dir.path().join(format!("data/avatars/ada/{shut}.png"))).unwrap();
    assert_eq!(hex::encode(Sha256::digest(&bytes)), shut);

    tools
        .answer(
            "make_face",
            &json!({ "style": "pixelbot", "parts": { "eyes": "soft" } }),
        )
        .await
        .unwrap();
    let still = avatar(&room).unwrap();
    assert_eq!(still.poses, None, "one-row eyes have nothing to shut");
    assert_eq!(kept_files(&dir), vec![format!("{}.png", still.hash)]);

    tools
        .answer("make_face", &json!({ "style": "sprouts" }))
        .await
        .unwrap();
    assert!(avatar(&room).unwrap().poses.is_some());
    tools
        .call("set_avatar", &json!({ "clear": true }))
        .await
        .unwrap();
    assert_eq!(kept_files(&dir), [] as [String; 0]);
}

/// A face kept before there were poses gets its eyes-shut picture when the
/// desk opens, the same one making it would have given, once: a second open
/// writes nothing. A face whose pose is on its record, or that has no
/// recipe, is left alone.
#[tokio::test]
async fn a_face_kept_before_poses_gets_one_when_the_desk_opens() {
    let (dir, room, tools) = room();
    tools
        .answer("make_face", &json!({ "style": "sprouts" }))
        .await
        .unwrap();
    let made = avatar(&room).unwrap();
    let shut = made.poses.clone().unwrap().shut;
    let mut before = room.persona("ada").unwrap();
    before.avatar.as_mut().unwrap().poses = None;
    crate::room::append_persona(room.log(), &before).unwrap();
    std::fs::remove_file(dir.path().join(format!("data/avatars/ada/{shut}.png"))).unwrap();

    give_faces_their_poses(room.log()).unwrap();
    let given = crate::room::roster(room.log())[0].avatar.clone().unwrap();
    assert_eq!(given, made, "the pose is the one making the face gave");
    assert!(
        dir.path()
            .join(format!("data/avatars/ada/{shut}.png"))
            .is_file()
    );
    let written = room.log().load(&crate::log::StreamId::Room).len();
    give_faces_their_poses(room.log()).unwrap();
    assert_eq!(room.log().load(&crate::log::StreamId::Room).len(), written);

    std::fs::write(dir.path().join("workspace/otter.png"), margined()).unwrap();
    tools
        .call("set_avatar", &json!({ "path": "otter.png" }))
        .await
        .unwrap();
    let written = room.log().load(&crate::log::StreamId::Room).len();
    give_faces_their_poses(room.log()).unwrap();
    assert_eq!(room.log().load(&crate::log::StreamId::Room).len(), written);
    assert_eq!(avatar(&room).unwrap().poses, None);
}
