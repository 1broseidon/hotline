use super::*;
use crate::contract::{McpPolicy, PolicyMode, SkillPolicy};

fn ask(arguments: Value) -> Ask {
    Ask::read(&arguments).unwrap()
}

fn made(arguments: Value, current: Option<&AvatarRecipe>) -> Face {
    compose(ask(arguments), current, "ada", FaceMaker::Teammate).unwrap()
}

fn persona(id: &str, bubble: Option<&str>) -> Persona {
    Persona {
        node: None,
        id: id.into(),
        name: "Ada".into(),
        goal: String::new(),
        avatar: None,
        team: None,
        bubble: bubble.map(str::to_string),
        backend_id: "hotline".into(),
        cwd: "/work".into(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        config_choices: BTreeMap::new(),
        effort_by_model: BTreeMap::new(),
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
    }
}

/// Every style draws a 512 px square with something in it, and its recipe
/// names every one of its parts, with the ones the seed left off as none.
#[test]
fn every_style_draws_a_square_face_and_a_whole_recipe() {
    for (kept, parsed) in STYLES.iter().zip(PARSED.iter()) {
        let face = made(json!({ "style": kept.id }), None);
        let picture = image::load_from_memory_with_format(&face.png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(picture.dimensions(), (EDGE, EDGE), "{}", kept.id);
        let drawn = picture.pixels().filter(|pixel| pixel[3] > 0).count();
        assert!(drawn > (EDGE * EDGE / 10) as usize, "{}: {drawn}", kept.id);
        let names: Vec<&String> = face.recipe.parts.keys().collect();
        let mut expected: Vec<&String> = parsed.parts.iter().map(|part| &part.name).collect();
        expected.sort();
        assert_eq!(names, expected, "{}", kept.id);
        assert!(
            face.recipe.parts.values().any(Option::is_some),
            "{}",
            kept.id
        );
        assert!(!face.recipe.colors.is_empty(), "{}", kept.id);
        assert_eq!(face.recipe.seed, "ada");
        assert_eq!(face.recipe.made_by, FaceMaker::Teammate);
    }
}

/// The same recipe draws the same face, and the recipe a face was made
/// from draws exactly that face again.
#[test]
fn a_recipe_draws_the_same_face_every_time() {
    let one = made(
        json!({ "style": "clay", "parts": { "eyes": "googly" } }),
        None,
    );
    let two = made(
        json!({ "style": "clay", "parts": { "eyes": "googly" } }),
        None,
    );
    assert_eq!(one.recipe, two.recipe);
    assert_eq!(one.png, two.png);
    assert_eq!(png(&one.recipe).unwrap(), one.png);
    let other = made(
        json!({ "style": "clay", "parts": { "eyes": "googly" }, "seed": "grace" }),
        None,
    );
    assert_eq!(other.recipe.parts["eyes"].as_deref(), Some("googly"));
    assert_ne!(other.png, one.png);
}

/// In the same style a call changes only what it names; a part can be left
/// off; a new style, or a new seed, starts afresh.
#[test]
fn the_same_style_keeps_what_is_not_named_and_another_starts_afresh() {
    let first = made(
        json!({ "style": "clay", "colors": { "body": "#69ACDE" } }),
        None,
    );
    assert_eq!(first.recipe.colors["body"], "#69acde");

    let glasses = made(
        json!({ "style": "clay", "parts": { "eyes": "googly", "mouth": "grin" } }),
        Some(&first.recipe),
    );
    for (name, variant) in &glasses.recipe.parts {
        match name.as_str() {
            "eyes" => assert_eq!(variant.as_deref(), Some("googly")),
            "mouth" => assert_eq!(variant.as_deref(), Some("grin")),
            _ => assert_eq!(variant, &first.recipe.parts[name], "{name}"),
        }
    }
    assert_eq!(glasses.not_drawn, Vec::<String>::new());
    assert_eq!(glasses.recipe.colors, first.recipe.colors);
    assert_eq!(glasses.recipe.seed, first.recipe.seed);

    let bare = made(
        json!({ "style": "clay", "parts": { "top": "none", "pattern": null } }),
        Some(&glasses.recipe),
    );
    assert_eq!(bare.recipe.parts["top"], None);
    assert_eq!(bare.recipe.parts["pattern"], None);
    assert_eq!(bare.recipe.parts["eyes"].as_deref(), Some("googly"));

    // A part on a part that has no place for it is said not to be drawn.
    let flat = made(
        json!({ "style": "clay", "parts": { "body": "gumdrop", "top": "horns" } }),
        Some(&bare.recipe),
    );
    assert_eq!(flat.recipe.parts["top"], None);
    assert_eq!(flat.not_drawn.len(), 1);
    assert!(
        flat.not_drawn[0].starts_with("top horns: it shows only with body bell, blob"),
        "{:?}",
        flat.not_drawn
    );

    let moods = made(json!({ "style": "moods" }), Some(&bare.recipe));
    assert_eq!(moods.recipe.style, "moods");
    assert!(moods.recipe.parts.contains_key("face"));
    assert!(!moods.recipe.parts.contains_key("body"));
    assert_eq!(moods.recipe, made(json!({ "style": "moods" }), None).recipe);

    let reseeded = made(
        json!({ "style": "clay", "seed": "grace" }),
        Some(&bare.recipe),
    );
    assert_eq!(
        reseeded.recipe,
        made(json!({ "style": "clay", "seed": "grace" }), None).recipe
    );
}

/// Every name is checked against the style and every colour is a #rrggbb;
/// a refusal names what would do.
#[test]
fn unknown_names_and_bad_colours_are_refused_with_what_would_do() {
    for (arguments, said) in [
        (json!({}), "needs a `style`"),
        (
            json!({ "style": "bottts" }),
            "The styles are: clay, critters",
        ),
        (json!({ "style": "adventurer" }), "no style called"),
        (
            json!({ "style": "clay", "parts": { "hat": "tall" } }),
            "Its parts are: body, eyes, mouth, pattern, top",
        ),
        (
            json!({ "style": "clay", "parts": { "eyes": "lasers" } }),
            "Use one of: big, dots",
        ),
        (
            json!({ "style": "clay", "parts": { "eyes": 7 } }),
            "has no variant 7",
        ),
        (
            json!({ "style": "clay", "parts": ["eyes"] }),
            "is an object",
        ),
        (
            json!({ "style": "clay", "colors": { "glow": "#ffffff" } }),
            "Its slots are: background, accent, body, ink",
        ),
        (
            json!({ "style": "clay", "colors": { "body": "red" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "clay", "colors": { "body": "#12345" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "clay", "colors": { "body": "#gggggg" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "clay", "colors": { "body": "69acde" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "clay", "seed": "x".repeat(MAX_SEED + 1) }),
            "A seed is",
        ),
        (json!({ "style": "clay", "seed": " " }), "A seed is"),
    ] {
        let Err(error) = Ask::read(&arguments) else {
            panic!("{arguments} was taken");
        };
        assert!(error.contains(said), "{arguments}: {error}");
    }
    let long = "x".repeat(10_000);
    let error = Ask::read(&json!({ "style": "clay", "parts": { long: "a" } }))
        .err()
        .unwrap();
    assert!(error.len() < 400, "a refusal repeats a bounded name");
}

/// The desk's face at setup is a creature style with named parts, in the
/// teammate's bubble colour, the rest from its id, and says the desk made
/// it.
#[test]
fn a_seeded_face_is_a_named_creature_in_the_bubble_colour() {
    for id in [
        "ada",
        "grace",
        "linus",
        "mira",
        "4f1c2b9e-0000-4000-8000-000000000000",
    ] {
        for bubble_name in [None, Some("pink")] {
            let teammate = persona(id, bubble_name);
            let face = seeded(&teammate).unwrap();
            let kept = STYLES
                .iter()
                .find(|kept| kept.id == face.recipe.style)
                .unwrap();
            assert!(!kept.person && kept.names == "named", "{}", kept.id);
            let slot = kept.seeded_in.unwrap();
            assert_eq!(face.recipe.colors[slot], bubble(&teammate).1, "{id}");
            assert_eq!(face.recipe.made_by, FaceMaker::Desk);
            assert_eq!(face.recipe.seed, id);
            assert_eq!(seeded(&teammate).unwrap().png, face.png);
        }
    }
}

/// A teammate's own bubble is worked out as the window works it out
/// (`ownBubble` in ui/src/bubbles.ts), and one the person picked wins.
#[test]
fn the_bubble_is_the_windows() {
    // "ada": ((97 * 31 + 100) * 31 + 97) % 7 == 3, the fourth hue: teal.
    assert_eq!(bubble(&persona("ada", None)), ("teal", "#48b8bc"));
    assert_eq!(bubble(&persona("ada", Some("pink"))), ("pink", "#c78ec4"));
    assert_eq!(bubble(&persona("ada", Some("grey"))), ("grey", "#a0a5ab"));
    // A name the desk does not know is its own again, as in the window.
    assert_eq!(bubble(&persona("ada", Some("gold"))), ("teal", "#48b8bc"));
}

/// What `face_options` says is what `make_face` takes: every style it lists
/// draws, and every part, variant and slot it lists for a style is taken.
#[test]
fn the_options_answer_what_make_face_takes() {
    let listed = styles_answer(&persona("ada", None));
    assert_eq!(listed["styles"].as_array().unwrap().len(), STYLES.len());
    assert_eq!(
        listed["bubble"],
        json!({ "name": "teal", "hex": "#48b8bc" })
    );
    assert_eq!(listed["current"], "none: people see your initial");
    for style in listed["styles"].as_array().unwrap() {
        let id = style["style"].as_str().unwrap();
        let answer = style_answer(id).unwrap();
        let mut parts = Map::new();
        for part in answer["parts"].as_array().unwrap() {
            let variants = part["variants"].as_array().unwrap();
            parts.insert(
                part["part"].as_str().unwrap().into(),
                variants[variants.len() - 1].clone(),
            );
        }
        let mut colors = Map::new();
        for slot in answer["colors"].as_array().unwrap() {
            colors.insert(slot["slot"].as_str().unwrap().into(), json!("#123456"));
        }
        let face = made(
            json!({ "style": id, "parts": parts, "colors": colors }),
            None,
        );
        for (name, variant) in &parts {
            let drawn = face.recipe.parts[name].as_deref() == variant.as_str();
            let said = face
                .not_drawn
                .iter()
                .any(|line| line.starts_with(&format!("{name} ")));
            assert!(drawn != said, "{id} {name}: {:?}", face.not_drawn);
        }
        assert!(!face.recipe.colors.is_empty(), "{id}");
        for (name, color) in &face.recipe.colors {
            assert_eq!(color, "#123456", "{id} {name}");
        }
    }
    assert!(style_answer("bottts").is_err());
    let clay = style_answer("clay").unwrap();
    let top = clay["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["part"] == "top")
        .unwrap();
    assert_eq!(top["drawnPercent"], 75.0);
    assert!(
        top["onlyWith"]["body"]
            .as_array()
            .is_some_and(|holding| !holding.contains(&json!("gumdrop")))
    );
}

/// The definitions read whole: no palette value dropped for not being a
/// #rrggbb, and no variant named as the word that leaves a part off.
#[test]
fn the_definitions_are_read_whole() {
    for (kept, parsed) in STYLES.iter().zip(PARSED.iter()) {
        let raw: Value = serde_json::from_str(kept.definition).unwrap();
        for slot in &parsed.slots {
            let values = raw["colors"][&slot.name]["values"]
                .as_array()
                .map_or(0, Vec::len);
            assert_eq!(slot.palette.len(), values, "{} {}", kept.id, slot.name);
        }
        for part in &parsed.parts {
            assert!(!part.variants.is_empty(), "{} {}", kept.id, part.name);
            assert!(
                !part.variants.iter().any(|variant| variant == LEFT_OFF),
                "{} {}",
                kept.id,
                part.name
            );
        }
    }
}

/// A face never moves: DiceBear only writes its animations when asked, and
/// the desk never asks.
#[test]
fn a_face_is_still() {
    for kept in &STYLES {
        let face = made(json!({ "style": kept.id }), None);
        let drawn = svg(&face.recipe).unwrap();
        assert!(!drawn.contains("@keyframes"), "{}", kept.id);
    }
}

/// Nothing here may need credit where it is shown: each style's own
/// definition names its licence, and every one drawn is CC0.
#[test]
fn every_style_drawn_is_cc0() {
    for kept in &STYLES {
        let definition: Value = serde_json::from_str(kept.definition).unwrap();
        assert_eq!(
            definition["meta"]["license"]["name"], "CC0 1.0",
            "{}",
            kept.id
        );
    }
}
